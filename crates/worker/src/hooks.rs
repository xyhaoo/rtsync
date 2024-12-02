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

pub trait JobHook{
    fn per_job(&self) -> Result<(), Box<dyn Error>>;
    fn per_exec(&self) -> Result<(), Box<dyn Error>>;
    fn post_exec(&self) -> Result<(), Box<dyn Error>>;
    fn post_success(&self) -> Result<(), Box<dyn Error>>;
    fn post_fail(&self) -> Result<(), Box<dyn Error>>;
}

pub struct EmptyHook{
    pub(crate) provider: Box<dyn MirrorProvider>,
}

impl JobHook for EmptyHook<> {
    fn per_job(&self) -> Result<(), Box<dyn Error>>{
        Ok(())
    }
    fn per_exec(&self) -> Result<(), Box<dyn Error>>{
        Ok(())
    }
    fn post_exec(&self) -> Result<(), Box<dyn Error>>{
        Ok(())
    }
    fn post_success(&self) -> Result<(), Box<dyn Error>>{
        Ok(())
    }
    fn post_fail(&self) -> Result<(), Box<dyn Error>>{
        Ok(())
    }
    
}