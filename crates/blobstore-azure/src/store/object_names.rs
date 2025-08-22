use azure_core::Pageable;
use azure_storage_blobs::container::operations::ListBlobsResponse;
use tokio::sync::Mutex;

use spin_core::async_trait;

pub struct AzureObjectNames {
    // The Mutex is used to make it Send
    stm: Mutex<Pageable<ListBlobsResponse, azure_core::error::Error>>,
    read_but_not_yet_returned: Vec<String>,
    end_stm_after_read_but_not_yet_returned: bool,
}

impl AzureObjectNames {
    pub fn new(stm: Pageable<ListBlobsResponse, azure_core::error::Error>) -> Self {
        Self {
            stm: Mutex::new(stm),
            read_but_not_yet_returned: Default::default(),
            end_stm_after_read_but_not_yet_returned: false,
        }
    }

    async fn read_impl(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        use futures::StreamExt;

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

        // TODO: do we need to prefix these with a prefix from somewhere or do they include it?
        let mut names: Vec<_> = chunk.blobs.blobs().map(|blob| blob.name.clone()).collect();
        let at_end = chunk.next_marker.is_none();

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
impl spin_factor_blobstore::ObjectNames for AzureObjectNames {
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
