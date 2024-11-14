use crate::hooks::EmptyHook;
use crate::provider::MirrorProvider;
use users;
pub struct ZfsHook<T: MirrorProvider> {
    empty_hook: EmptyHook<T>,
    z_pool: String,
}

// impl ZfsHook {
//     fn new(provider: Box<dyn MirrorProvider>, z_pool: String) -> Self {
//         ZfsHook{
//             empty_hook: EmptyHook{
//                 provider,
//             },
//             z_pool,
//         }
//     }
//     fn print_help_message(&self) {
//         let zfs_dataset = format!("{}/{}", self.z_pool, self.empty_hook.provider.name()).to_lowercase();
//         
//         
//     }
//     
//     // 检查工作目录是否为ZFS数据集
//     // fn per_job()
// }