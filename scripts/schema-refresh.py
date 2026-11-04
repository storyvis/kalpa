#!/usr/bin/env python3
"""Generate documented OAS 3.0 client projections from pinned official sources.

Default is offline generation; --check verifies committed outputs and hashes;
--fetch acquires current official sources before regeneration. No API keys used.
"""
import argparse
import copy
import datetime
import gzip
import hashlib
import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1] / "crates/libgen/specs"
UPSTREAM = ROOT / "upstream"
SOURCES = {
    "openai": "https://raw.githubusercontent.com/openai/openai-openapi/main/openapi.json",
    "gemini": "https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta",
    "vertex": "https://aiplatform.googleapis.com/$discovery/rest?version=v1",
    "falai": "https://fal.ai/docs/api-reference/inference-api/openapi/queue.json",
}


def dump(value):
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def fetch(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        body = response.read(32 * 1024 * 1024 + 1)
    if len(body) > 32 * 1024 * 1024:
        raise ValueError("upstream specification exceeds 32 MiB")
    return body


def refresh_sources():
    manifest = {}
    snapshots = {}
    for name, url in SOURCES.items():
        revision = None
        if name == "openai":
            revision = json.loads(fetch("https://api.github.com/repos/openai/openai-openapi/commits/main"))["sha"]
            url = url.replace("/main/", f"/{revision}/")
        raw = fetch(url)
        source = json.loads(raw)
        revision = revision or source.get("revision", source.get("info", {}).get("version"))
        snapshots[name] = gzip.compress(raw, mtime=0)
        manifest[name] = {
            "url": url, "sha256": hashlib.sha256(raw).hexdigest(),
            "fetched_at": datetime.date.today().isoformat(), "revision": revision,
            "bytes": len(raw),
        }
    license_text = fetch(manifest["openai"]["url"].replace("openapi.json", "LICENSE"))
    # Do not overwrite pinned files until every network read succeeded.
    for name, compressed in snapshots.items():
        (UPSTREAM / f"{name}.json.gz").write_bytes(compressed)
    (UPSTREAM / "LICENSE.openai").write_bytes(license_text)
    (UPSTREAM / "manifest.json").write_text(dump(manifest))


def load(name, manifest):
    raw = gzip.decompress((UPSTREAM / f"{name}.json.gz").read_bytes())
    assert hashlib.sha256(raw).hexdigest() == manifest[name]["sha256"], name
    assert len(raw) == manifest[name]["bytes"], name
    return json.loads(raw)


class Projection:
    def __init__(self, name, source, provenance):
        self.name = name
        self.source = source
        self.provenance = provenance
        self.upstream = source.get("schemas", source.get("components", {}).get("schemas", {}))
        self.schemas = {}
        self.paths = {}

    def resolve(self, schema):
        schema = copy.deepcopy(schema)
        if "$ref" in schema:
            return self.resolve(self.upstream[schema["$ref"].split("/")[-1]])
        if "allOf" in schema:
            result = {"type": "object", "properties": {}, "required": []}
            for item in schema["allOf"]:
                item = self.resolve(item)
                result["properties"].update(item.get("properties", {}))
                result["required"].extend(item.get("required", []))
            return result
        return schema

    def leaf(self, schema):
        """OAS3.1/Discovery scalar -> OAS3.0, lenient enums/formats."""
        schema = self.resolve(schema)
        union = schema.get("anyOf", schema.get("oneOf"))
        if union:
            choices = [self.resolve(s) for s in union]
            # Select the supported scalar branch. Object/array unions need an
            # explicit field projection, never an invented catch-all type.
            choices = [s for s in choices if s.get("type") in ("string", "integer", "number", "boolean")]
            assert choices, schema
            result = self.leaf(choices[0])
            if any(s.get("type") == "null" for s in union):
                result["nullable"] = True
            return result
        result = {k: copy.deepcopy(schema[k]) for k in ("type", "description", "deprecated", "minimum", "maximum", "minLength", "maxLength") if k in schema}
        typ = result.get("type")
        if isinstance(typ, list):
            assert len([t for t in typ if t != "null"]) == 1, schema
            result["type"] = next(t for t in typ if t != "null")
            result["nullable"] = "null" in typ
        if result.get("type") == "any":
            result.pop("type")
        if result.get("type") == "array":
            result["items"] = self.leaf(schema["items"])
        if result.get("type") == "object":
            if "properties" in schema:
                result["properties"] = {k: self.leaf(v) for k, v in schema["properties"].items()}
            if "additionalProperties" in schema:
                extra = schema["additionalProperties"]
                result["additionalProperties"] = self.leaf(extra) if isinstance(extra, dict) else extra
        return result

    def object(self, name, original, fields, required=()):
        source = self.resolve({"$ref": original}) if isinstance(original, str) else self.resolve(original)
        properties = source["properties"]
        result = {"type": "object", "properties": {}}
        for field, target in fields.items():
            assert field in properties, (self.name, original, field)
            value = properties[field]
            if target:
                if self.resolve(value).get("type") == "array":
                    projected = {"type": "array", "items": {"$ref": f"#/components/schemas/{target}"}}
                else:
                    projected = {"$ref": f"#/components/schemas/{target}"}
            else:
                projected = self.leaf(value)
            result["properties"][field] = projected
        if required:
            assert set(required) <= set(fields)
            result["required"] = list(required)
        result["x-kalpa-source-schema"] = original if isinstance(original, str) else "inline schema"
        self.schemas[name] = result
        return result

    def operation(self, path, method, operation_id, request, response, parameters=()):
        operation = {"operationId": operation_id, "responses": {"200": {
            "description": "Successful provider response", "content": {
                "application/json": {"schema": {"$ref": f"#/components/schemas/{response}"} if isinstance(response, str) else response}
            }
        }}}
        if request:
            operation["requestBody"] = {"required": True, "content": {"application/json": {"schema": {"$ref": f"#/components/schemas/{request}"} if isinstance(request, str) else request}}}
        if parameters:
            operation["parameters"] = [{"name": p, "in": "path", "required": True, "schema": {"type": "string"}} for p in parameters]
        self.paths.setdefault(path, {})[method] = operation
        return operation

    def document(self, server, security):
        return {
            "openapi": "3.0.3",
            "info": {"title": f"Kalpa {self.name} supported-operation projection", "version": str(self.provenance["revision"]), "description": "Generated from official source by scripts/schema-refresh.py. This is a compatibility projection, not the full upstream API. See specs/README.md for retained fields and transformations."},
            "servers": [{"url": server}], "paths": self.paths,
            "security": [{"providerAuth": []}],
            "components": {"schemas": self.schemas, "securitySchemes": {"providerAuth": security}},
            "x-kalpa-upstream": self.provenance,
        }


def fields(names):
    return dict.fromkeys(names.split())


def openai(source, provenance):
    p = Projection("openai", source, provenance)
    p.object("ChatCompletionRequest", "CreateChatCompletionRequest", {**fields("model temperature max_tokens max_completion_tokens"), "messages": "Message"}, ("model", "messages"))
    message = p.object("Message", "ChatCompletionRequestUserMessage", fields("role content"))
    message["properties"]["role"]["enum"] = [p.resolve(p.upstream[f"ChatCompletionRequest{role.title()}Message"])["properties"]["role"]["enum"][0] for role in ("system", "user", "assistant")]
    p.object("ChatCompletionResponse", "CreateChatCompletionResponse", {"id": None, "choices": "Choice", "usage": "CompletionUsage"})
    choices = p.resolve(p.upstream["CreateChatCompletionResponse"])["properties"]["choices"]["items"]
    p.object("Choice", choices, {"index": None, "message": "AssistantMessage"})
    p.object("AssistantMessage", "ChatCompletionResponseMessage", fields("role content"))
    p.object("CompletionUsage", "CompletionUsage", fields("prompt_tokens completion_tokens total_tokens"))
    p.object("ImageGenerationRequest", "CreateImageRequest", fields("model prompt n size"), ("model", "prompt"))
    p.object("ImageGenerationResponse", "ImagesResponse", {"created": None, "data": "Image", "usage": "Usage"})
    p.object("Image", "Image", fields("url b64_json"))
    p.object("Usage", "ImageGenUsage", fields("input_tokens output_tokens total_tokens input_tokens_details"))
    p.object("VideoGenerationRequest", "CreateVideoJsonBody", fields("model prompt size seconds"), ("model", "prompt"))
    p.object("VideoGenerationResponse", "VideoResource", fields("id object model status progress created_at size seconds"))
    for path, op_id, request, response in (("/chat/completions", "chatCompletions", "ChatCompletionRequest", "ChatCompletionResponse"), ("/images/generations", "createImage", "ImageGenerationRequest", "ImageGenerationResponse"), ("/videos", "createVideo", "VideoGenerationRequest", "VideoGenerationResponse")):
        assert "post" in source["paths"][path]
        p.operation("/v1" + path, "post", op_id, request, response)
    return p.document("https://api.openai.com", {"type": "http", "scheme": "bearer"})


def google(source, provenance, vertex=False):
    name = "vertex" if vertex else "gemini"
    p = Projection(name, source, provenance)
    prefix = "GoogleCloudAiplatformV1" if vertex else ""
    schema_names = {"GenerateRequest": "GenerateContentRequest", "GenerateResponse": "GenerateContentResponse", "InlineData": "Blob"} if vertex else {}
    def obj(local, props, required=()):
        p.object(local, prefix + schema_names.get(local, local), props, required)
    request = "GenerateRequest" if vertex else "GenerateContentRequest"
    response = "GenerateResponse" if vertex else "GenerateContentResponse"
    blob = "InlineData" if vertex else "Blob"
    obj(request, {"contents": "Content", "generationConfig": "GenerationConfig", **({"safetySettings": "SafetySetting"} if vertex else {})}, ("contents",))
    obj("Content", {"role": None, "parts": "Part"})
    obj("Part", {"text": None, "inlineData": blob, "fileData": "FileData"})
    obj(blob, fields("mimeType data"), ("mimeType", "data"))
    obj("FileData", fields("mimeType fileUri"), ("mimeType", "fileUri"))
    obj("GenerationConfig", fields("temperature maxOutputTokens topP topK stopSequences responseMimeType responseModalities"))
    obj(response, {"candidates": "Candidate", "usageMetadata": "UsageMetadata"})
    obj("Candidate", {"content": "Content", "finishReason": None, **({"safetyRatings": "SafetyRating", "citationMetadata": "CitationMetadata"} if vertex else {})})
    obj("UsageMetadata", fields("promptTokenCount candidatesTokenCount totalTokenCount"))
    if vertex:
        obj("SafetySetting", fields("category threshold"))
        obj("SafetyRating", fields("category probability"))
        obj("CitationMetadata", fields("citations"))
        p.object("OperationResponse", "GoogleLongrunningOperation", {"name": None, "done": None, "error": "OperationError", "response": None, "metadata": None})
        p.object("OperationError", "GoogleRpcStatus", fields("code message"))
        p.schemas.update(json.loads((ROOT / "overlays/vertex-models.json").read_text())["schemas"])
        methods = source["resources"]["projects"]["resources"]["locations"]["resources"]["publishers"]["resources"]["models"]["methods"]
        path = "/v1/projects/{projectId}/locations/{location}/publishers/google/models/{modelId}"
        params = ("projectId", "location", "modelId")
        for method, req, resp in (("generateContent", request, response), ("predict", "PredictRequest", "PredictResponse"), ("predictLongRunning", "VideoPredictRequest", "OperationResponse")):
            assert methods[method]["httpMethod"] == "POST"
            p.operation(path + ":" + method, "post", method, req, resp, params)
        assert "get" in source["resources"]["projects"]["resources"]["locations"]["resources"]["operations"]["methods"]
        p.operation("/v1/projects/{projectId}/locations/{location}/operations/{operationId}", "get", "getOperation", None, "OperationResponse", ("projectId", "location", "operationId"))
    else:
        methods = source["resources"]["models"]["methods"]
        assert methods["generateContent"]["request"]["$ref"] == request
        p.operation("/v1beta/models/{model}:generateContent", "post", "generateContent", request, response, ("model",))
        # The old stream method incorrectly decoded a streaming body as one JSON
        # object. Keep only the non-streaming method this library implements.
    auth = {"type": "http", "scheme": "bearer"} if vertex else {"type": "apiKey", "in": "header", "name": "x-goog-api-key"}
    return p.document(source["rootUrl"].rstrip("/"), auth)


def falai(source, provenance):
    p = Projection("falai", source, provenance)
    p.object("FalQueueSubmitResponse", "SubmitResponse", fields("request_id status_url response_url cancel_url"), ("request_id",))
    p.object("FalLogEntry", "LogEntry", fields("message timestamp"), ("message",))
    for path, method, operation_id, request, response, params in (
        ("/{model_id}", "post", "submitToQueue", p.leaf(source["components"]["schemas"]["ModelInput"]), "FalQueueSubmitResponse", ("model_id",)),
        ("/{model_id}/requests/{request_id}/status", "get", "getQueueStatus", None, {"type": "object"}, ("model_id", "request_id")),
        ("/{model_id}/requests/{request_id}", "get", "getQueueResult", None, p.leaf(source["components"]["schemas"]["ModelOutput"]), ("model_id", "request_id")),
    ):
        upstream_path = path.replace("{model_id}", "{owner}/{app}")
        assert method in source["paths"][upstream_path]
        op = p.operation(path, method, operation_id, request, response, params)
        if operation_id == "getQueueStatus":
            op["parameters"].append({"name": "logs", "in": "query", "schema": {"type": "string"}})
    return p.document("https://queue.fal.run", {"type": "apiKey", "in": "header", "name": "Authorization"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fetch", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.fetch and args.check:
        parser.error("--fetch and --check are mutually exclusive")
    if args.fetch:
        refresh_sources()
    manifest = json.loads((UPSTREAM / "manifest.json").read_text())
    for name, project in (("openai", openai), ("gemini", google), ("vertex", lambda s, p: google(s, p, True)), ("falai", falai)):
        result = dump(project(load(name, manifest), manifest[name]))
        path = ROOT / f"{name}.json"
        if args.check:
            assert path.read_text() == result, f"stale generated projection: {path}"
        else:
            path.write_text(result)
        action = "checked" if args.check else "generated"
        print(f"{name}: verified official source and {action} projection")


if __name__ == "__main__":
    main()
