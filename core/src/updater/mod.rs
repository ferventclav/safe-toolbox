//! Updates from every source, checked at once, with only the safe ones ticked.

pub mod checker;
pub mod dev;
pub mod model;
pub mod nvidia;
pub mod path_health;
pub mod policy;
pub mod release_notes;
pub mod run;
pub mod smoke;
pub mod sources;
pub mod storage;
pub mod versions;
pub mod windows_update;
pub mod winget;
pub mod worker;
