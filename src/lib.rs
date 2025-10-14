mod methods;
pub use methods::get_available_method;

mod ssh;
pub use ssh::{scp, ssh};

mod magic;
pub use magic::*;

pub mod utils;

mod frame;

mod local_proxy;
pub use local_proxy::{Args, init_local_proxy};

mod remote_proxy;
pub use remote_proxy::init_remote_proxy;
