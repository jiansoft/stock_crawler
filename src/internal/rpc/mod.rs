pub mod client;
pub mod server;

pub mod stock {
    include!("stock.rs");
}

pub mod basic {
    include!("basic.rs");
}

pub mod control {
    include!("control.rs");
}
