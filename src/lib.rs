mod methods;
pub use methods::{BaseMethod, get_available_method};

mod ssh;
pub use ssh::{scp, ssh};

mod magic;
pub mod utils;
pub use magic::*;

mod frame;
pub use frame::{Frame, FrameType, Header, Protocol};
