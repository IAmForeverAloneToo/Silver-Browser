# Wycheproof's vectors

Copied from [C2SP/wycheproof](https://github.com/C2SP/wycheproof), under
its Apache License 2.0 (`LICENSE` beside this file), at commit

```text
65ca494727a52944c204441c860b039e6786b99b (2026-11-11)
```

| File | From |
| --- | --- |
| `aes_gcm_test.json` | `testvectors_v1/aes_gcm_test.json` |
| `ecdsa_secp256r1_sha256_p1363_test.json` | `testvectors_v1/ecdsa_secp256r1_sha256_p1363_test.json` |
| `ecdsa_secp256r1_sha512_p1363_test.json` | `testvectors_v1/ecdsa_secp256r1_sha512_p1363_test.json` |
| `ecdsa_secp384r1_sha512_p1363_test.json` | `testvectors_v1/ecdsa_secp384r1_sha512_p1363_test.json` |
| `ecdsa_secp521r1_sha512_p1363_test.json` | `testvectors_v1/ecdsa_secp521r1_sha512_p1363_test.json` |
| `ecdh_secp256r1_webcrypto_test.json` | `testvectors_v1/ecdh_secp256r1_webcrypto_test.json` |
| `rsa_oaep_2048_sha1_mgf1sha1_test.json` | `testvectors_v1/rsa_oaep_2048_sha1_mgf1sha1_test.json` |
| `rsa_oaep_2048_sha256_mgf1sha256_test.json` | `testvectors_v1/rsa_oaep_2048_sha256_mgf1sha256_test.json` |
| `rsa_oaep_2048_sha384_mgf1sha384_test.json` | `testvectors_v1/rsa_oaep_2048_sha384_mgf1sha384_test.json` |
| `rsa_oaep_2048_sha512_mgf1sha512_test.json` | `testvectors_v1/rsa_oaep_2048_sha512_mgf1sha512_test.json` |
| `rsa_oaep_3072_sha256_mgf1sha256_test.json` | `testvectors_v1/rsa_oaep_3072_sha256_mgf1sha256_test.json` |
| `rsa_pkcs1_1024_sig_gen_test.json` | `testvectors_v1/rsa_pkcs1_1024_sig_gen_test.json` |
| `rsa_pkcs1_1536_sig_gen_test.json` | `testvectors_v1/rsa_pkcs1_1536_sig_gen_test.json` |
| `rsa_pkcs1_2048_sig_gen_test.json` | `testvectors_v1/rsa_pkcs1_2048_sig_gen_test.json` |
| `rsa_pss_2048_sha1_mgf1_20_test.json` | `testvectors_v1/rsa_pss_2048_sha1_mgf1_20_test.json` |
| `rsa_pss_2048_sha256_mgf1_0_test.json` | `testvectors_v1/rsa_pss_2048_sha256_mgf1_0_test.json` |
| `rsa_pss_2048_sha256_mgf1_32_test.json` | `testvectors_v1/rsa_pss_2048_sha256_mgf1_32_test.json` |
| `rsa_pss_2048_sha384_mgf1_48_test.json` | `testvectors_v1/rsa_pss_2048_sha384_mgf1_48_test.json` |
| `rsa_pss_4096_sha512_mgf1_64_test.json` | `testvectors_v1/rsa_pss_4096_sha512_mgf1_64_test.json` |
| `rsa_signature_2048_sha256_test.json` | `testvectors_v1/rsa_signature_2048_sha256_test.json` |
| `rsa_signature_2048_sha384_test.json` | `testvectors_v1/rsa_signature_2048_sha384_test.json` |
| `rsa_signature_2048_sha512_test.json` | `testvectors_v1/rsa_signature_2048_sha512_test.json` |
| `rsa_signature_3072_sha256_test.json` | `testvectors_v1/rsa_signature_3072_sha256_test.json` |
| `rsa_signature_4096_sha512_test.json` | `testvectors_v1/rsa_signature_4096_sha512_test.json` |
| `mlkem_768_keygen_seed_test.json` | `testvectors_v1/mlkem_768_keygen_seed_test.json` |
| `mlkem_1024_keygen_seed_test.json` | `testvectors_v1/mlkem_1024_keygen_seed_test.json` |
| `mldsa_44_sign_seed_test.json` | `testvectors_v1/mldsa_44_sign_seed_test.json` |
| `mldsa_44_verify_test.json` | `testvectors_v1/mldsa_44_verify_test.json` |
| `mldsa_65_sign_seed_test.json` | `testvectors_v1/mldsa_65_sign_seed_test.json` |
| `mldsa_65_verify_test.json` | `testvectors_v1/mldsa_65_verify_test.json` |
| `mldsa_87_sign_seed_test.json` | `testvectors_v1/mldsa_87_sign_seed_test.json` |
| `mldsa_87_verify_test.json` | `testvectors_v1/mldsa_87_verify_test.json` |
| `mlkem_1024_encaps_test.json` | `testvectors_v1/mlkem_1024_encaps_test.json` |
| `mlkem_1024_test.json` | `testvectors_v1/mlkem_1024_test.json` |
| `mlkem_768_encaps_test.json` | `testvectors_v1/mlkem_768_encaps_test.json` |
| `mlkem_768_test.json` | `testvectors_v1/mlkem_768_test.json` |
| `LICENSE` | `LICENSE` |

Their SHA-256s are in `MANIFEST`, which a test holds every file here to
(but `MANIFEST` and this README): the vector files hold test keys, which
gitleaks would take for secrets, and `/.gitleaks.toml` allows this
directory's `*_test.json` files alone, upstream's bytes and nothing else.

`tests/wycheproof.rs` runs every vector through Web Crypto's own calls
(G255): an import, then each operation as a page would make it.
