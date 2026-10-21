use crate::host::{Host, Request, json_body};
use serde_json::json;
use ui::screens::diff::panel::{BlobReply, FullError, FullFileRequest, FullText};

pub fn read_full_file(host: &dyn Host, request: &FullFileRequest) -> Result<FullText, FullError> {
    let result = host
        .fetch(&Request::post(
            "git-cat-file",
            Some(json!({
                "cwd": request.cwd,
                "requests": request.requests,
                "maxObjectBytes": 5_242_880,
            })),
        ))
        .and_then(|response| json_body(&response))
        .and_then(|body| serde_json::from_value::<Vec<BlobReply>>(body).map_err(Into::into));
    match result {
        Ok(replies) => request.decode(replies),
        Err(error) => {
            eprintln!("codex-gpui: cannot load diff file: {error:#}");
            Err(FullError::Failed)
        }
    }
}
