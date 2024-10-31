#[global_allocator]
#[cfg(not(miri))]
static GLOBAL: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;
