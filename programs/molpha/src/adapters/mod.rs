pub mod pyth;
pub mod switchboard;

// Re-export main traits but not encoding modules to avoid conflicts
pub use pyth::{PriceStatus, PythCompatible, PythPrice, ToPythPrice};
pub use switchboard::{SwitchboardCompatible, SwitchboardResult, ToSwitchboardResult};

