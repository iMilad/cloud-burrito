//! Small additive response envelope. IDs are opaque strings in the webview.

use serde_json::{json, Value};

use crate::aws::{context::VerifiedSession, AwsContext};

#[derive(Default)]
pub(crate) struct RequestEnvelope {
    id: Option<String>,
    context: Option<Value>,
}

impl RequestEnvelope {
    pub(crate) fn from_params(params: &Value) -> Result<Self, &'static str> {
        let id = match params.get("request_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(id))
                if !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)) =>
            {
                Some(id.clone())
            }
            _ => return Err("request_id must be a short ASCII identifier"),
        };
        Ok(Self { id, context: None })
    }

    pub(crate) fn bind(&mut self, context: &AwsContext, session: &VerifiedSession) {
        self.context = Some(json!({
            "context_id": context.id().to_string(),
            "provider_revision": session.provider_revision.to_string(),
            "settings_revision": session.snapshot.settings_revision.to_string(),
            "profile": session.snapshot.profile,
            "account_id": session.identity.account_id,
            "region": session.snapshot.region,
        }));
    }

    pub(crate) fn attach(self, mut response: Value) -> Value {
        let mut metadata = self.context.unwrap_or_else(|| {
            json!({
                "context_id": null, "provider_revision": null, "settings_revision": null,
                "profile": null, "account_id": null, "region": null,
            })
        });
        metadata["id"] = json!(self.id);
        if !response.is_object() {
            response = json!({"render": "raw_json", "data": response});
        }
        response["_request"] = metadata;
        response
    }
}
