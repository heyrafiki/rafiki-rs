//! Transport, authentication, error and retry behavior tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use heyrafiki::{
    BookingInput, CareFormat, Client, Currency, Error, ListParams, PaymentSource,
    PreauthorizationDecisionInput, WebhookEndpointInput, WebhookEventType, WriteOptions,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};

fn test_client(server: &MockServer) -> Client {
    Client::builder("sk_test_not_a_real_key")
        .base_url(format!("{}/v1", server.uri()))
        .build()
        .expect("test client")
}

fn practitioner_list() -> Value {
    json!({
        "object": "list",
        "data": [{
            "id": "prc_2481",
            "object": "practitioner",
            "name": "Synthetic Practitioner",
            "profession": "Counselling Psychologist",
            "location": { "city": "Nairobi", "country": "Kenya" },
            "session_fee": { "amount": 350_000, "currency": "KES" }
        }],
        "has_more": false
    })
}

fn booking() -> Value {
    json!({
        "id": "bkg_1001",
        "object": "booking",
        "session_id": "ses_1001",
        "practitioner_id": "prc_2481",
        "starts_at": "2026-08-10T07:00:00Z",
        "ends_at": "2026-08-10T08:00:00Z",
        "timezone": "Africa/Nairobi",
        "format": "online",
        "status": "reserved",
        "payment_source": "self_pay"
    })
}

fn claim_valuation() -> Value {
    json!({
        "id": "clm_1001:2026-08-28T10:00:00Z",
        "object": "claim_valuation",
        "claim_id": "clm_1001",
        "valuation_at": "2026-08-28T10:00:00Z",
        "currency": "KES",
        "status": "approved",
        "amount": {
            "billed": 350_000,
            "payer_liability": 300_000,
            "patient_responsibility": 50_000,
            "adjustment": 0,
            "remitted": 300_000,
            "settled": 300_000,
            "outstanding": 0
        },
        "policy": {
            "reference": "pol_care_v1",
            "version": "1.0"
        },
        "events": [
            {
                "sequence": 1,
                "type": "submitted",
                "effective_at": "2026-08-28T08:00:00Z",
                "recorded_at": "2026-08-28T08:00:05Z",
                "previous_status": "draft",
                "next_status": "submitted",
                "reason_code": null,
                "evidence_references": []
            },
            {
                "sequence": 2,
                "type": "approved",
                "effective_at": "2026-08-28T09:30:00Z",
                "recorded_at": "2026-08-28T09:30:02Z",
                "previous_status": "submitted",
                "next_status": "approved",
                "reason_code": "covered_benefit",
                "evidence_references": ["doc_eval_001"]
            }
        ]
    })
}

#[tokio::test]
async fn sends_server_side_auth_and_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/practitioners"))
        .and(query_param("limit", "5"))
        .and(header("authorization", "Bearer sk_test_not_a_real_key"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(practitioner_list()))
        .expect(1)
        .mount(&server)
        .await;

    let response = test_client(&server)
        .practitioners()
        .list(ListParams::new(5).expect("valid limit"))
        .await
        .expect("practitioner list");

    assert_eq!(response.data[0].id, "prc_2481");
}

#[tokio::test]
async fn idempotent_write_retries_503_with_the_same_key_and_body() {
    let server = MockServer::start().await;
    let input = BookingInput {
        practitioner_id: "prc_2481".into(),
        starts_at: "2026-08-10T07:00:00Z".into(),
        ends_at: "2026-08-10T08:00:00Z".into(),
        format: CareFormat::Online,
        payment_source: PaymentSource::SelfPay,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/bookings"))
        .and(header("idempotency-key", "booking-demo-0001"))
        .and(body_json(&input))
        .respond_with(SequenceResponder {
            calls: Arc::clone(&calls),
            first: ResponseTemplate::new(503)
                .insert_header("retry-after", "0")
                .set_body_json(json!({
                    "error": {
                        "code": "service_unavailable",
                        "message": "Try again later.",
                        "docs": "https://docs.heyrafiki.space/errors"
                    }
                })),
            then: ResponseTemplate::new(201).set_body_json(booking()),
        })
        .mount(&server)
        .await;

    let response = test_client(&server)
        .bookings()
        .create(
            &input,
            WriteOptions::new("booking-demo-0001").expect("valid idempotency key"),
        )
        .await
        .expect("retried booking");

    assert_eq!(response.id, "bkg_1001");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn non_idempotent_write_does_not_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after", "0")
                .set_body_json(json!({
                    "error": {
                        "code": "service_unavailable",
                        "message": "Try again later.",
                        "docs": "https://docs.heyrafiki.space/errors"
                    }
                })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let error = test_client(&server)
        .webhook_endpoints()
        .create(&WebhookEndpointInput {
            url: "https://example.com/webhooks".into(),
            events: vec![WebhookEventType::SandboxPing],
        })
        .await
        .expect_err("503 must be returned without an unsafe retry");

    assert!(matches!(error, Error::Api(api) if api.status.as_u16() == 503));
}

#[tokio::test]
async fn exposes_the_stable_error_code_and_request_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/claims/clm_missing"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("x-request-id", "req_123")
                .set_body_json(json!({
                    "error": {
                        "code": "resource_not_found",
                        "message": "Claim not found.",
                        "docs": "https://docs.heyrafiki.space/errors"
                    }
                })),
        )
        .mount(&server)
        .await;

    let error = test_client(&server)
        .claims()
        .retrieve("clm_missing")
        .await
        .expect_err("missing Claim");

    match error {
        Error::Api(api) => {
            assert_eq!(api.code, "resource_not_found");
            assert_eq!(api.request_id.as_deref(), Some("req_123"));
            assert!(api.retry_after.is_none());
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn validates_configuration_and_does_not_debug_api_keys() {
    let client = Client::new("sk_test_not_a_real_key").expect("client");
    assert!(!format!("{client:?}").contains("sk_test_not_a_real_key"));
    assert!(Client::new("  ").is_err());
    assert!(
        Client::builder("sk_test_key")
            .base_url("http://api.example.com/v1")
            .build()
            .is_err()
    );
    assert!(WriteOptions::new("short").is_err());
    assert!(ListParams::new(0).is_err());
}

#[test]
fn serializes_contract_enums_exactly() {
    assert_eq!(
        serde_json::to_value(Currency::Kes).expect("currency"),
        "KES"
    );
    let denied = PreauthorizationDecisionInput::Denied {
        reason_codes: vec!["benefit_exhausted".into()],
        policy_reference: "policy:demo".into(),
        policy_version: "1".into(),
        evidence_references: vec!["evidence:demo".into()],
    };
    assert_eq!(
        serde_json::to_value(denied).expect("decision")["outcome"],
        "denied"
    );
}

#[tokio::test]
async fn reproduces_claim_valuation_at_explicit_cutoff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/claims/clm_1001/valuation"))
        .and(query_param("valuation_at", "2026-08-28T10:00:00Z"))
        .and(header("authorization", "Bearer sk_test_not_a_real_key"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(claim_valuation()))
        .expect(1)
        .mount(&server)
        .await;

    let valuation = test_client(&server)
        .claims()
        .valuation("clm_1001", "2026-08-28T10:00:00Z")
        .await
        .expect("claim valuation");

    assert_eq!(valuation.id, "clm_1001:2026-08-28T10:00:00Z");
    assert_eq!(valuation.claim_id, "clm_1001");
    assert_eq!(valuation.status, heyrafiki::ClaimStatus::Approved);
    assert_eq!(valuation.amount.billed, 350_000);
    assert_eq!(valuation.amount.settled, 300_000);
    assert_eq!(valuation.events.len(), 2);
    assert_eq!(
        valuation.events[0].event_type,
        heyrafiki::ClaimValuationEventType::Submitted
    );
    assert_eq!(
        valuation.events[1].event_type,
        heyrafiki::ClaimValuationEventType::Approved
    );
}

struct SequenceResponder {
    calls: Arc<AtomicUsize>,
    first: ResponseTemplate,
    then: ResponseTemplate,
}

impl Respond for SequenceResponder {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.first.clone()
        } else {
            self.then.clone()
        }
    }
}
