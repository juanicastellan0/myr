pub mod connection_manager;
pub mod profiles;
pub mod query_runner;
pub mod results_buffer;
pub mod schema_cache;

#[must_use]
pub fn domain_name() -> &'static str {
    "myr-core"
}

#[cfg(test)]
mod tests {
    use super::domain_name;

    #[test]
    fn domain_name_is_stable() {
        assert_eq!(domain_name(), "myr-core");
    }
}
