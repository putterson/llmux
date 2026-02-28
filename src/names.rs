use petname::{Generator, Petnames};

/// Generate a random petname with 3 words separated by hyphens.
pub fn generate() -> String {
    let mut rng = rand::thread_rng();
    Petnames::default()
        .generate(&mut rng, 3, "-")
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()[..12].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn petname_is_three_words() {
        let name = generate();
        assert_eq!(name.split('-').count(), 3);
    }
}
