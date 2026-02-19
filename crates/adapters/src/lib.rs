#[must_use]
pub fn adapter_name() -> &'static str {
    "myr-adapters"
}

#[cfg(test)]
mod tests {
    use super::adapter_name;

    #[test]
    fn adapter_name_is_stable() {
        assert_eq!(adapter_name(), "myr-adapters");
    }
}
