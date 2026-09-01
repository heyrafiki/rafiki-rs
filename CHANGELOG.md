# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows Semantic Versioning.

## [0.1.0-beta.2] - 2026-08-28

- Added typed async client method `Claims::valuation` for `GET /claims/{claim_id}/valuation` (`getClaimValuation`) with historical cutoff parameter `valuation_at`.
- Updated embedded OpenAPI contract snapshot to the canonical 31-operation definition.
- Added typed `ClaimValuation`, `ClaimValuationAmount`, `ClaimValuationEvent`, and `ClaimValuationEventType` models.
- Added deterministic fixture-driven client and contract conformance tests.

## [0.1.0-beta.1] - 2026-08-09

- Added typed async clients for every operation in the published Heyrafiki API contract.
- Added explicit idempotency, API errors and bounded retries for `429` and `503` responses.
- Added contract provenance and conformance tests.

[0.1.0-beta.2]: https://github.com/heyrafiki/rafiki-rs/releases/tag/v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/heyrafiki/rafiki-rs/releases/tag/v0.1.0-beta.1
