// 查找镜像配置的差异，这对于热加载配置文件很重要
// 注意：只支持[[mirrors]]部分

use crate::config::MirrorConfig;

#[derive(Debug)]
struct SortableMirrorConfig(MirrorConfig);

impl SortableMirrorConfig {
    pub(crate) fn clone(&self) -> Self {
        SortableMirrorConfig(self.0.clone())
    }
    pub(crate) fn deep_eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Ord for SortableMirrorConfig {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.name.cmp(&other.0.name)
    }
}
impl PartialOrd for SortableMirrorConfig {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for SortableMirrorConfig {
    fn eq(&self, other: &Self) -> bool {
        self.0.name == other.0.name
    }
}
impl Eq for SortableMirrorConfig {}

impl From<MirrorConfig> for SortableMirrorConfig {
    fn from(value: MirrorConfig) -> Self {
        SortableMirrorConfig(value)
    }
}


const DIFF_DELETE: u8 = 0;
const DIFF_ADD: u8 = 1;
const DIFF_MODIFY: u8 = 2;

// 镜像配置差异单元
struct MirrorCfgTrans{
    diff_op: u8,
    mir_cfg: MirrorConfig
}

impl MirrorCfgTrans {
    fn string(&self) -> String{
        let op = match self.diff_op {
            DIFF_DELETE => "del".to_string(),
            _ => "add".to_string(),
        };
        format!("{{{}, {}}}", op, self.mir_cfg.name.clone().unwrap_or(String::new()))
    }
}

// diff_mirror_config查找old_list和new_list之间的差异
// 它返回一系列操作
// 如果这些操作应用于old_list，则可以获得new_list等价。
fn diff_mirror_config(old_list: &Vec<MirrorConfig>, new_list: &Vec<MirrorConfig>) -> Vec<MirrorCfgTrans>{
    let mut operations: Vec<MirrorCfgTrans> = vec![];
    let mut o_list: Vec<SortableMirrorConfig> = Vec::with_capacity(old_list.len());
    let mut n_list: Vec<SortableMirrorConfig> = Vec::with_capacity(new_list.len());

    o_list.extend(old_list.iter().map(|x|x.clone().into()));
    n_list.extend(new_list.iter().map(|x|x.clone().into()));

    // 首先确保old_list new_list是排好序的
    o_list.sort();
    n_list.sort();

    if o_list.len() != 0 && n_list.len() != 0{
        // 在两个list中插入尾节点作为最大节点
        let (last_ord, last_new) =
            (o_list[o_list.len()-1].clone(), n_list[n_list.len()-1].clone());

        // 😅
        let mut max_name = last_ord.0.name.clone().unwrap_or(String::new());
        if last_new.0.name.as_ref().unwrap_or(&String::new()) > last_ord.0.name.as_ref().unwrap_or(&String::new()){
            max_name = last_new.0.name.unwrap_or(String::new())
        }
        let nil = MirrorConfig{
            name: Some(format!("~{}",max_name)),
            ..Default::default()
        };
        if nil.name.clone().unwrap() <= max_name{
            panic!("Nil.Name应该比maxName大");
        }
        o_list.push(SortableMirrorConfig(nil.clone()));
        n_list.push(SortableMirrorConfig(nil));

        // 遍历两个list，找到差别
        let (mut i,mut j) = (0,0);
        while  i<o_list.len() && j<n_list.len(){
            let (o, n) = (o_list[i].clone(), n_list[j].clone());
            if n.0.name < o.0.name{
                operations.push(MirrorCfgTrans{diff_op: DIFF_ADD, mir_cfg: n.0.clone()});
                j+=1;
            }else if o.0.name < n.0.name{
                operations.push(MirrorCfgTrans{diff_op: DIFF_DELETE, mir_cfg: o.0.clone()});
                i+=1;
            }else {
                if !o.deep_eq(&n) {
                    operations.push(MirrorCfgTrans{diff_op: DIFF_MODIFY, mir_cfg: n.0.clone()});
                }
                i+=1;
                j+=1;
            }
        }
    }else {
        for i in 0..o_list.len(){
            operations.push(MirrorCfgTrans{diff_op: DIFF_DELETE, mir_cfg: o_list[i].0.clone()})
        }
        for i in 0..n_list.len(){
            operations.push(MirrorCfgTrans{diff_op: DIFF_ADD, mir_cfg: n_list[i].0.clone()})
        }
    }
    operations
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use super::*;
    #[test]
    fn test_equal_configs(){
        let old_list = vec![
            MirrorConfig{name: Some(String::from("debian")), ..Default::default()},
            MirrorConfig{name: Some(String::from("debian-security")), ..Default::default()},
            MirrorConfig{name: Some(String::from("fedora")), ..Default::default()},
            MirrorConfig{name: Some(String::from("archlinux")), ..Default::default()},
            MirrorConfig{name: Some(String::from("AOSP")), ..Default::default()},
            MirrorConfig{name: Some(String::from("ubuntu")), ..Default::default()},
        ];
        let mut new_list = Vec::with_capacity(old_list.len());
        new_list.extend(old_list.iter().cloned());
        let difference = diff_mirror_config(&old_list, &new_list);
        assert_eq!(difference.len(), 0);
    }
    #[test]
    fn test_empty_old(){
        let new_list = vec![
            MirrorConfig{name: Some(String::from("debian")), ..Default::default()},
            MirrorConfig{name: Some(String::from("debian-security")), ..Default::default()},
            MirrorConfig{name: Some(String::from("fedora")), ..Default::default()},
            MirrorConfig{name: Some(String::from("archlinux")), ..Default::default()},
            MirrorConfig{name: Some(String::from("AOSP")), ..Default::default()},
            MirrorConfig{name: Some(String::from("ubuntu")), ..Default::default()},
        ];
        let old_list: Vec<MirrorConfig> = vec![];
        let difference = diff_mirror_config(&old_list, &new_list);
        assert_eq!(difference.len(), new_list.len());
    }
    #[test]
    fn test_empty_new(){
        let old_list = vec![
            MirrorConfig{name: Some(String::from("debian")), ..Default::default()},
            MirrorConfig{name: Some(String::from("debian-security")), ..Default::default()},
            MirrorConfig{name: Some(String::from("fedora")), ..Default::default()},
            MirrorConfig{name: Some(String::from("archlinux")), ..Default::default()},
            MirrorConfig{name: Some(String::from("AOSP")), ..Default::default()},
            MirrorConfig{name: Some(String::from("ubuntu")), ..Default::default()},
        ];
        let new_list: Vec<MirrorConfig> = vec![];
        let difference = diff_mirror_config(&old_list, &new_list);
        assert_eq!(difference.len(), old_list.len());
    }
    #[test]
    fn test_diff_list_name(){
        let old_list = vec![
            MirrorConfig{name: Some(String::from("debian")), ..Default::default()},
            MirrorConfig{name: Some(String::from("debian-security")), ..Default::default()},
            MirrorConfig{name: Some(String::from("fedora")), ..Default::default()},
            MirrorConfig{name: Some(String::from("archlinux")), ..Default::default()},
            MirrorConfig{name: Some(String::from("AOSP")), 
                env: Some(vec![("REPO".to_string(), "/usr/bin/repo".to_string())].into_iter().collect()), 
                ..Default::default()},
            MirrorConfig{name: Some(String::from("ubuntu")), ..Default::default()},
        ];
        let mut new_list = vec![
            MirrorConfig{name: Some(String::from("debian")), ..Default::default()},
            MirrorConfig{name: Some(String::from("debian-cd")), ..Default::default()},
            MirrorConfig{name: Some(String::from("archlinuxcn")), ..Default::default()},
            MirrorConfig{name: Some(String::from("AOSP")),
                env: Some(vec![("REPO".to_string(), "/usr/local/bin/aosp-repo".to_string())].into_iter().collect()),
                ..Default::default()},
            MirrorConfig{name: Some(String::from("ubuntu-ports")), ..Default::default()},
        ];
        
        let difference = diff_mirror_config(&old_list, &new_list);
        
        let mut old_list = old_list.into_iter()
            .map(|x|x.into())
            .collect::<Vec<SortableMirrorConfig>>();
        old_list.sort();
        
        let mut empty_list: Vec<MirrorConfig> = vec![];
        for o in old_list.iter(){
            let mut keep = true;
            for op in difference.iter(){
                if (op.diff_op == DIFF_DELETE || op.diff_op == DIFF_MODIFY) && 
                    op.mir_cfg.name.as_ref().unwrap_or(&String::new()) == o.0.name.as_ref().unwrap_or(&String::new()){
                    keep = false;
                    break
                }
            }
            if keep {
                empty_list.push(o.0.clone());
            }
        }
        
        for op in difference.iter(){
            if op.diff_op == DIFF_ADD || op.diff_op == DIFF_MODIFY {
                empty_list.push(op.mir_cfg.clone());
            }
        }
        let mut empty_list = empty_list.into_iter()
            .map(|x|x.into())
            .collect::<Vec<SortableMirrorConfig>>();
        empty_list.sort();
        let mut new_list = new_list.into_iter()
            .map(|x|x.into())
            .collect::<Vec<SortableMirrorConfig>>();
        new_list.sort();

        assert_eq!(empty_list, new_list);
    }
}