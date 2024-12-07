use std::error::Error;
use crate::provider::MirrorProvider;
/*
hooks to exec before/after syncing
                                                                        failed
                              +------------------ post-fail hooks -------------------+
                              |                                                      |
 job start -> pre-job hooks --v-> pre-exec hooks --> (syncing) --> post-exec hooks --+---------> post-success --> end
                                                                                       success
*/

pub(crate) trait JobHook{
    fn per_job(&self) -> Result<(), Box<dyn Error>> {Ok(())}
    fn pre_exec(&self) -> Result<(), Box<dyn Error>> {Ok(())}
    fn post_exec(&self) -> Result<(), Box<dyn Error>> {Ok(())}
    fn post_success(&self) -> Result<(), Box<dyn Error>> {Ok(())}
    fn post_fail(&self) -> Result<(), Box<dyn Error>> {Ok(())}
}
pub(crate) struct EmptyHook<T: Clone>{
    pub(crate) provider: Box<dyn MirrorProvider<ContextStoreVal=T>>,
}

impl<T: Clone> JobHook for EmptyHook<T> {}