/// NStock EPS crawler。
pub mod eps;
/// NStock 即時報價 crawler。
pub mod price;

const HOST: &str = "www.nstock.tw";

/// NStock 來源命名空間標記型別。
pub struct NStock {}
