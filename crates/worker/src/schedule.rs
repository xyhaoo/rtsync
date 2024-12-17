// use std::collections::HashMap;
// use std::sync::{Arc, Mutex};
// use std::time::{Duration, SystemTime};
// use skiplist::{SkipList, SkipMap};
// use log::{debug, warn};
//
// pub struct ScheduleQueue {
//     list: Arc<Mutex<SkipMap<SystemTime, Box<dyn Job>>>>,
//     jobs: Arc<Mutex<HashMap<String, bool>>>,
// }
//
// pub struct JobScheduleInfo {
//     pub job_name: String,
//     pub next_scheduled: SystemTime,
// }
//
// pub trait Job {
//     fn name(&self) -> String;
// }
//
// impl ScheduleQueue {
//     pub fn new() -> Self {
//         ScheduleQueue {
//             list: Arc::new(Mutex::new(SkipMap::new())),
//             jobs: Arc::new(Mutex::new(HashMap::new())),
//         }
//     }
//
//     pub fn get_jobs(&self) -> Vec<JobScheduleInfo> {
//         let mut jobs = Vec::new();
//         let list = self.list.lock().unwrap();
//
//         for (sched_time, job) in list.iter() {
//             jobs.push(JobScheduleInfo {
//                 job_name: job.name(),
//                 next_scheduled: sched_time.clone(),
//             });
//         }
//
//         jobs
//     }
//
//     pub fn add_job(&self, sched_time: SystemTime, job: Box<dyn Job>) {
//         let mut jobs = self.jobs.lock().unwrap();
//         let job_name = job.name();
//
//         // Remove existing job if already scheduled
//         if jobs.contains_key(&job_name) {
//             warn!("Job {} already scheduled, removing the existing one", job_name);
//             self.remove(&job_name);
//         }
//
//         jobs.insert(job_name.clone(), true);
//
//         let mut list = self.list.lock().unwrap();
//         list.insert(sched_time, job);
//
//         debug!("Added job {} @ {:?}", job_name, sched_time);
//     }
//
//     pub fn pop(&self) -> Option<Box<dyn Job>> {
//         let mut list = self.list.lock().unwrap();
//
//         if let Some((sched_time, job)) = list.iter().next() {
//             if sched_time < &SystemTime::now() {
//                 list.remove(sched_time);
//                 self.jobs.lock().unwrap().remove(&job.name());
//                 debug!("Popped out job {} @ {:?}", job.name(), sched_time);
//                 return Some(*job);
//             }
//         }
//         None
//     }
//
//     pub fn remove(&self, name: &str) -> bool {
//         let mut list = self.list.lock().unwrap();
//         let mut jobs = self.jobs.lock().unwrap();
//
//         if let Some(pos) = list.iter().position(|(_, job)| job.name() == name) {
//             list.remove(&list.keys().nth(pos).unwrap());
//             jobs.remove(name);
//             return true;
//         }
//         false
//     }
// }
//
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use std::time::Duration;
//
//     struct TestJob {
//         name: String,
//     }
//
//     impl Job for TestJob {
//         fn name(&self) -> String {
//             self.name.clone()
//         }
//     }
//
//     #[test]
//     fn test_schedule_queue() {
//         let queue = ScheduleQueue::new();
//         let job1 = Box::new(TestJob { name: "job1".to_string() });
//
//         let sched_time = SystemTime::now() + Duration::from_secs(10);
//         queue.add_job(sched_time, job1);
//
//         let jobs = queue.get_jobs();
//         assert_eq!(jobs.len(), 1);
//         assert_eq!(jobs[0].job_name, "job1");
//
//         if let Some(job) = queue.pop() {
//             assert_eq!(job.name(), "job1");
//         }
//     }
// }
