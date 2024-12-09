use std::error::Error;
use std::sync::{Arc, Mutex};
use crate::base_provider::BaseProvider;
use crate::context::Context;
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
    type ContextStoreVal: Clone;
    fn per_job(&self, 
               _working_dir: String, 
               _provider_name: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn pre_exec(&self, 
                _provider_name: String,
                _log_dir: String, 
                _log_file: String, 
                _working_dir: String, 
                _context: &Arc<Mutex<Option<Context<Self::ContextStoreVal>>>>) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_exec(&self, 
                 _context: &Arc<Mutex<Option<Context<Self::ContextStoreVal>>>>, 
                 _provider_name: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_success(&self, 
                    _provider_name: String,
                    _working_dir: String,
                    _upstream: String,
                    _log_dir: String,
                    _log_file: String) 
        -> Result<(), Box<dyn Error>> {Ok(())}
    
    fn post_fail(&self,
                 _provider_name: String,
                 _working_dir: String,
                 _upstream: String,
                 _log_dir: String,
                 _log_file: String,
                 _context: &Arc<Mutex<Option<Context<Self::ContextStoreVal>>>>) 
        -> Result<(), Box<dyn Error>> {Ok(())}
}
pub(crate) struct EmptyHook{
    // pub(crate) provider: Box<dyn MirrorProvider<ContextStoreVal=T>>,
}
