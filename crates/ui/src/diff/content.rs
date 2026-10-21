#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobRequest {
    pub path: String,
    pub oid: Option<String>,
    pub fallback_to_disk: bool,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BlobReply {
    Success { lines: Vec<String> },
    Error { error: BlobError },
}

#[derive(serde::Deserialize)]
pub struct BlobError {
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone)]
pub struct FullFileRequest {
    pub cwd: String,
    pub requests: Vec<BlobRequest>,
    pub before: Option<usize>,
    pub after: Option<usize>,
}

#[derive(Clone)]
pub struct FullText {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FullError {
    Unavailable,
    Failed,
}

impl FullFileRequest {
    pub fn decode(&self, mut replies: Vec<BlobReply>) -> Result<FullText, FullError> {
        if replies.len() != self.requests.len()
            || (self.before.is_some() && self.before == self.after)
        {
            return Err(FullError::Failed);
        }
        let mut take = |index: Option<usize>| match index {
            None => Ok(Vec::new()),
            Some(index) => match replies.get_mut(index) {
                Some(BlobReply::Success { lines }) => Ok(std::mem::take(lines)),
                Some(BlobReply::Error { error })
                    if matches!(error.kind.as_str(), "not-found" | "too-large") =>
                {
                    Err(FullError::Unavailable)
                }
                _ => Err(FullError::Failed),
            },
        };
        Ok(FullText {
            before: take(self.before)?,
            after: take(self.after)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> FullFileRequest {
        FullFileRequest {
            cwd: "/repo".into(),
            requests: (0..2)
                .map(|_| BlobRequest {
                    path: "file.txt".into(),
                    oid: None,
                    fallback_to_disk: false,
                })
                .collect(),
            before: Some(0),
            after: Some(1),
        }
    }

    #[test]
    fn incomplete_or_reused_responses_fail() {
        let mut request = request();
        assert_eq!(request.decode(vec![]).err(), Some(FullError::Failed));
        request.after = request.before;
        let replies = (0..2)
            .map(|_| BlobReply::Success {
                lines: vec!["line\n".into()],
            })
            .collect();
        assert_eq!(request.decode(replies).err(), Some(FullError::Failed));
    }
}
