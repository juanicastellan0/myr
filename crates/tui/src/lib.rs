#[must_use]
pub fn ui_name() -> &'static str {
    "myr-tui"
}

#[cfg(test)]
mod tests {
    use super::ui_name;

    #[test]
    fn ui_name_is_stable() {
        assert_eq!(ui_name(), "myr-tui");
    }
}
