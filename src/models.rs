use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub value: i64,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreateItemRequest {
    pub name: String,
    pub value: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ItemEvent {
    #[serde(rename = "item_created")]
    Created { id: String, name: String, value: i64, created_at: String },
}

impl Item {
    pub fn validate_name(name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("name cannot be empty".to_string());
        }
        if name.len() > 255 {
            return Err("name cannot exceed 255 characters".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_name_empty() {
        let result = Item::validate_name("");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "name cannot be empty");
    }

    #[test]
    fn test_validate_name_whitespace() {
        let result = Item::validate_name("   ");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_name_too_long() {
        let long_name = "a".repeat(256);
        let result = Item::validate_name(&long_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_name_valid() {
        let result = Item::validate_name("valid name");
        assert!(result.is_ok());
    }
}
