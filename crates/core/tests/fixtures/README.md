# Synthetic RSA test keys

These public, disposable 2048-bit fixtures were generated solely for the RS256
PKCS#1/PKCS#8 interoperability test. They are not credentials and must never be
used for an account, deployment, certificate or production signature.

```sh
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out test-rsa-pkcs8.pem
openssl rsa -in test-rsa-pkcs8.pem -traditional -out test-rsa-pkcs1.pem
```
