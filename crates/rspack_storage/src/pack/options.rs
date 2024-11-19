#[derive(Debug, Clone)]
pub struct PackOptions {
  pub buckets: usize,
  pub max_pack_size: usize,
  pub expires: u64,
}
