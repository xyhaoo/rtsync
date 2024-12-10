use std::error::Error;
use std::sync::{Arc, Mutex};
use crate::config::MirrorConfig;
use crate::context::Context;
use crate::hooks::{JobHook};
use crate::provider::MirrorProvider;

#[cfg(not(target_os = "linux"))]
#[derive(Debug, Clone)]
pub(crate) struct BtrfsSnapshotHook{}
#[cfg(not(target_os = "linux"))]
impl BtrfsSnapshotHook {
    pub(crate) fn new(
        _snapshot_path: &str,
        _mirror_config: MirrorConfig)
        -> BtrfsSnapshotHook
    {
        BtrfsSnapshotHook{}
    }
}

#[cfg(not(target_os = "linux"))]
impl JobHook for BtrfsSnapshotHook{

    fn per_job(&self,
               _working_dir: String,
               provider_name: String) 
        -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    
    fn pre_exec(&self, 
                _provider_name: String, 
                _log_dir: String, 
                _log_file: String, 
                _working_dir: String, 
                _context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>> 
    {
        Ok(())
    }
    
    fn post_exec(&self,
                 _context: &Arc<Mutex<Option<Context>>>, 
                 provider_name: String) 
        -> Result<(), Box<dyn Error>> 
    {
        Ok(())
    }
    
    fn post_success(&self,
                    _provider_name: String,
                    _working_dir: String,
                    _upstream: String,
                    _log_dir: String,
                    _log_file: String) -> Result<(), Box<dyn Error>> {
        Ok(())
    }

    fn post_fail(&self,
                 _provider_name: String,
                 _working_dir: String,
                 _upstream: String,
                 _log_dir: String,
                 _log_file: String,
                 _context: &Arc<Mutex<Option<Context>>>) 
        -> Result<(), Box<dyn Error>> 
    {
        Ok(())
    }
}

