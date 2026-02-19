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
