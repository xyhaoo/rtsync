use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};
use chrono::{DateTime, Utc};
use skiplist::skipmap::SkipMap;
use log::{debug, warn};
use serde::Serialize;
use crate::job::MirrorJob;
// jobs的调度队列

#[derive(Debug, Clone)]
pub struct ScheduleQueue {
    pub(crate) list: Arc<Mutex<SkipMap<DateTime<Utc>, MirrorJob>>>,
    pub(crate) jobs: Arc<Mutex<HashMap<String, bool>>>,
}

pub struct JobScheduleInfo {
    pub job_name: String,
    pub next_scheduled: DateTime<Utc>,
}



impl ScheduleQueue {
    pub fn new() -> Self {
        ScheduleQueue {
            list: Arc::new(Mutex::new(SkipMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_jobs(&self) -> Vec<JobScheduleInfo> {
        let mut jobs = Vec::new();
        let list = self.list.lock().await;

        for (sched_time, job) in list.iter() {
            jobs.push(JobScheduleInfo {
                job_name: job.name(),
                next_scheduled: sched_time.clone(),
            });
        }
        jobs
    }

    pub async fn add_job(&self, sched_time: DateTime<Utc>, job: MirrorJob) {
        let mut jobs = self.jobs.lock().await;
        let mut list = self.list.lock().await;
        
        let job_name = job.name();

        // 移除已经存在于调度队列的job
        if jobs.contains_key(&job_name) {
            warn!("job {} 已被安排, 准备将其删除", job_name);
            self.remove(&job_name, &mut list, &mut jobs);
        }

        jobs.insert(job_name.clone(), true);
        list.insert(sched_time, job);

        debug!("添加了 job {} @ {:?}", job_name, sched_time);
    }

    // 如果第一个任务的同步时间比现在早，将其弹出
    pub async fn pop(&self) -> Option<MirrorJob> {
        let mut list = self.list.lock().await;
        
        if let Some((sched_time, job)) = list.front() {
            let sched_time = *sched_time;
            let job_name = job.name();
            if sched_time < Utc::now() {
                let ret = list.pop_front().unwrap();
                self.jobs.lock().await.remove(&job_name);
                debug!("移出 job {} @ {:?}", job_name, sched_time);
                return Some(ret.1);
            }
        }
        None
    }

    pub fn remove(&self, 
                  name: &str, 
                  list_lock: &mut MutexGuard<SkipMap<DateTime<Utc>, MirrorJob>>, 
                  jobs_lock: &mut MutexGuard<HashMap<String, bool>>) 
        -> bool 
    {
        let mut to_remove = None;
        for entry in list_lock.iter() {
            if entry.1.name() == name {
                to_remove = Some(entry.0.clone());
                break;
            }
        }
        if let Some(key) = to_remove {
            list_lock.remove(&key);
            jobs_lock.remove(name);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use internal::logger::init_logger;
    use crate::cmd_provider::{CmdConfig, CmdProvider};

    #[tokio::test]
    async fn test_popping_on_empty_schedule() {
        let schedule = ScheduleQueue::new();
        let job = schedule.pop().await;
        assert!(job.is_none());
    }
    #[tokio::test]
    async fn test_adding_some_jobs(){
        let schedule = ScheduleQueue::new();
        let c = CmdConfig{
            name: "schedule_test".to_string(),
            ..CmdConfig::default()
        };
        let provider = CmdProvider::new(c).await.unwrap();
        let job = MirrorJob::new(Box::new(provider));
        let sched = Utc::now() + Duration::seconds(1);
        
        schedule.add_job(sched, job.clone()).await;
        assert!(schedule.pop().await.is_none());
        tokio::time::sleep(core::time::Duration::from_millis(1200)).await;
        assert_eq!(schedule.pop().await.unwrap(), job);
    }
    
    #[tokio::test]
    async fn test_adding_one_job_twice(){
        init_logger(true, true, false);
        
        let schedule = ScheduleQueue::new();
        let c = CmdConfig{
            name: "schedule_test".to_string(),
            ..CmdConfig::default()
        };
        let provider = CmdProvider::new(c).await.unwrap();
        let job = MirrorJob::new(Box::new(provider));
        let sched = Utc::now() + Duration::seconds(1);
        
        schedule.add_job(sched, job.clone()).await;
        schedule.add_job(sched + Duration::seconds(1), job.clone()).await;
        
        assert!(schedule.pop().await.is_none());
        tokio::time::sleep(core::time::Duration::from_millis(1200)).await;
        assert!(schedule.pop().await.is_none());
        tokio::time::sleep(core::time::Duration::from_millis(1200)).await;
        assert_eq!(schedule.pop().await.unwrap(), job);
        
    }
    
    #[tokio::test]
    async fn test_removing_jobs(){
        let schedule = ScheduleQueue::new();
        let c = CmdConfig{
            name: "schedule_test".to_string(),
            ..CmdConfig::default()
        };
        let provider = CmdProvider::new(c).await.unwrap();
        let job = MirrorJob::new(Box::new(provider));
        let sched = Utc::now() + Duration::seconds(1);
    
        schedule.add_job(sched, job.clone()).await;
        let mut list_lock = schedule.list.lock().await;
        let mut jobs_lock = schedule.jobs.lock().await;
        assert_eq!(schedule.remove("something", &mut list_lock, &mut jobs_lock), false);
        assert_eq!(schedule.remove("schedule_test", &mut list_lock, &mut jobs_lock), true);
        drop((list_lock, jobs_lock));
        tokio::time::sleep(core::time::Duration::from_millis(1200)).await;
        assert!(schedule.pop().await.is_none());
    }
}
