// Simple API test that can run without Docker
// Tests the models and basic validation

#[test]
fn test_item_validation() {
    use home_task::Item;

    // Valid name
    assert!(Item::validate_name("Test Item").is_ok());

    // Empty name
    assert!(Item::validate_name("").is_err());
    assert_eq!(
        Item::validate_name("").unwrap_err(),
        "name cannot be empty"
    );

    // Whitespace only
    assert!(Item::validate_name("   ").is_err());

    // Too long
    let long_name = "a".repeat(256);
    assert!(Item::validate_name(&long_name).is_err());
}

#[test]
fn test_create_item_request_serialization() {
    use home_task::CreateItemRequest;

    // With value
    let json = r#"{"name":"Test","value":42}"#;
    let req: CreateItemRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "Test");
    assert_eq!(req.value, Some(42));

    // Without value
    let json = r#"{"name":"Test"}"#;
    let req: CreateItemRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name, "Test");
    assert_eq!(req.value, None);
}

#[test]
fn test_item_event_serialization() {
    use home_task::ItemEvent;

    let json = r#"{"type":"item_created","id":"123","name":"Test","value":42,"created_at":"2024-01-01T00:00:00Z"}"#;
    let event: ItemEvent = serde_json::from_str(json).unwrap();

    match event {
        ItemEvent::Created { id, name, value, .. } => {
            assert_eq!(id, "123");
            assert_eq!(name, "Test");
            assert_eq!(value, 42);
        }
    }
}
