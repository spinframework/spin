use aws_sdk_s3::config::http::HttpResponse as AwsHttpResponse;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::list_objects_v2;
use aws_smithy_async::future::pagination_stream::PaginationStream;
use tokio::sync::Mutex;

use anyhow::Result;
use spin_core::async_trait;

pub struct S3ObjectNames {
    stm: Mutex<
        PaginationStream<
            Result<
                list_objects_v2::ListObjectsV2Output,
                SdkError<list_objects_v2::ListObjectsV2Error, AwsHttpResponse>,
            >,
        >,
    >,
    read_but_not_yet_returned: Vec<String>,
    end_stm_after_read_but_not_yet_returned: bool,
}

impl S3ObjectNames {
    pub fn new(
        stm: PaginationStream<
            Result<
                list_objects_v2::ListObjectsV2Output,
                SdkError<list_objects_v2::ListObjectsV2Error, AwsHttpResponse>,
            >,
        >,
    ) -> Self {
        Self {
            stm: Mutex::new(stm),
            read_but_not_yet_returned: Default::default(),
            end_stm_after_read_but_not_yet_returned: false,
        }
    }

    async fn read_impl(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        let len: usize = len.try_into().unwrap();

        // If we have names outstanding, send that first.  (We are allowed to send less than len,
        // and so sending all pending stuff before paging, rather than trying to manage a mix of
        // pending stuff with newly retrieved chunks, simplifies the code.)
        if !self.read_but_not_yet_returned.is_empty() {
            if self.read_but_not_yet_returned.len() <= len {
                // We are allowed to send all pending names
                let to_return = self.read_but_not_yet_returned.drain(..).collect();
                return Ok((to_return, self.end_stm_after_read_but_not_yet_returned));
            } else {
                // Send as much as we can. The rest remains in the pending buffer to send,
                // so this does not represent end of stream.
                let to_return = self.read_but_not_yet_returned.drain(0..len).collect();
                return Ok((to_return, false));
            }
        }

        // Get one chunk and send as much as we can of it. Aagin, we don't need to try to
        // pack the full length here - we can send chunk by chunk.

        let Some(chunk) = self.stm.get_mut().next().await else {
            return Ok((vec![], false));
        };
        let chunk = chunk.unwrap();

        let at_end = chunk.continuation_token().is_none();
        let mut names: Vec<_> = chunk
            .contents
            .unwrap_or_default()
            .into_iter()
            .flat_map(|blob| blob.key)
            .collect();

        if names.len() <= len {
            // We can send them all!
            Ok((names, at_end))
        } else {
            // We have more names than we can send in this response. Send what we can and
            // stash the rest.
            let to_return: Vec<_> = names.drain(0..len).collect();
            self.read_but_not_yet_returned = names;
            self.end_stm_after_read_but_not_yet_returned = at_end;
            Ok((to_return, false))
        }
    }
}

#[async_trait]
impl spin_factor_blobstore::ObjectNames for S3ObjectNames {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        self.read_impl(len).await // Separate function because rust-analyser gives better intellisense when async_trait isn't in the picture!
    }

    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64, bool)> {
        // TODO: there is a question (raised as an issue on the repo) about the required behaviour
        // here. For now I assume that skipping fewer than `num` is allowed as long as we are
        // honest about it. Because it is easier that is why.
        let (skipped, at_end) = self.read_impl(num).await?;
        Ok((skipped.len().try_into().unwrap(), at_end))
    }
}
