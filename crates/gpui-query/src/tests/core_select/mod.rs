use std::sync::Arc;

use crate::core::{MappedQueryResource, SelectTransform};

#[test]
fn select_transform_apply_identity() {
    let transform = SelectTransform::new(|x: &i32| *x);
    assert_eq!(transform.apply(&42), 42);
    assert_eq!(transform.apply(&0), 0);
    assert_eq!(transform.apply(&-1), -1);
}

#[test]
fn select_transform_apply_count() {
    let transform = SelectTransform::new(|v: &Vec<String>| v.len());
    assert_eq!(transform.apply(&vec!["a".to_string(), "b".to_string()]), 2);
    assert_eq!(transform.apply(&Vec::<String>::new()), 0);
}

#[test]
fn select_transform_apply_projection() {
    let transform = SelectTransform::new(|user: &(String, u32)| user.0.clone());
    assert_eq!(transform.apply(&("Alice".to_string(), 30)), "Alice");
}

#[test]
fn select_transform_apply_uppercase() {
    let transform = SelectTransform::new(|s: &String| s.to_uppercase());
    assert_eq!(transform.apply(&"hello".to_string()), "HELLO");
}

#[test]
fn select_transform_clone_shares_transform() {
    let transform = SelectTransform::new(|x: &i32| x * 2);
    let cloned = transform.clone();
    assert_eq!(transform.apply(&5), 10);
    assert_eq!(cloned.apply(&5), 10);
    assert_eq!(transform.apply(&3), cloned.apply(&3));
}

#[test]
fn select_transform_different_types() {
    let len_transform = SelectTransform::new(|s: &String| s.len());
    assert_eq!(len_transform.apply(&"hello".to_string()), 5);

    let empty_check = SelectTransform::new(|v: &Vec<i32>| v.is_empty());
    assert!(empty_check.apply(&vec![]));
    assert!(!empty_check.apply(&vec![1]));
}

#[test]
fn mapped_resource_new_with_data() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.len());
    let mapped = MappedQueryResource::<_, usize, ()>::new(Some(Arc::new(vec![1, 2, 3])), transform);
    assert!(mapped.has_data());
    assert_eq!(mapped.data(), Some(3));
}

#[test]
fn mapped_resource_new_without_data() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.len());
    let mapped: MappedQueryResource<Vec<i32>, usize, ()> =
        MappedQueryResource::new(None, transform);
    assert!(!mapped.has_data());
    assert_eq!(mapped.data(), None);
}

#[test]
fn mapped_resource_update_source_to_some() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.iter().sum::<i32>());
    let mut mapped: MappedQueryResource<Vec<i32>, i32, ()> =
        MappedQueryResource::new(None, transform);
    assert_eq!(mapped.data(), None);

    mapped.update_source(Some(Arc::new(vec![1, 2, 3])));
    assert!(mapped.has_data());
    assert_eq!(mapped.data(), Some(6));
}

#[test]
fn mapped_resource_update_source_to_none() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.len());
    let mut mapped: MappedQueryResource<Vec<i32>, usize, ()> =
        MappedQueryResource::new(Some(Arc::new(vec![1, 2, 3])), transform);
    assert_eq!(mapped.data(), Some(3));

    mapped.update_source(None);
    assert!(!mapped.has_data());
    assert_eq!(mapped.data(), None);
}

#[test]
fn mapped_resource_update_source_replaces_previous() {
    let transform = SelectTransform::new(|v: &Vec<String>| v.join(", "));
    let mut mapped: MappedQueryResource<Vec<String>, String, ()> =
        MappedQueryResource::new(Some(Arc::new(vec!["a".to_string()])), transform);
    assert_eq!(mapped.data(), Some("a".to_string()));

    mapped.update_source(Some(Arc::new(vec!["x".to_string(), "y".to_string()])));
    assert_eq!(mapped.data(), Some("x, y".to_string()));
}

#[test]
fn mapped_resource_data_applies_transform_lazily() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.len());

    let mut mapped: MappedQueryResource<Vec<i32>, usize, ()> =
        MappedQueryResource::new(Some(Arc::new(vec![1, 2])), transform);
    assert_eq!(mapped.data(), Some(2));

    assert_eq!(mapped.data(), Some(2));

    mapped.update_source(Some(Arc::new(vec![1, 2, 3])));
    assert_eq!(mapped.data(), Some(3));
}

#[test]
fn mapped_resource_clone_is_independent() {
    let transform = SelectTransform::new(|v: &Vec<i32>| v.len());
    let mut mapped: MappedQueryResource<Vec<i32>, usize, ()> =
        MappedQueryResource::new(Some(Arc::new(vec![1, 2, 3])), transform);

    let mut cloned = mapped.clone();
    assert_eq!(cloned.data(), Some(3));

    mapped.update_source(Some(Arc::new(vec![1])));
    assert_eq!(mapped.data(), Some(1));
    assert_eq!(cloned.data(), Some(3), "clone should be independent");

    cloned.update_source(Some(Arc::new(vec![4, 5, 6, 7])));
    assert_eq!(cloned.data(), Some(4));
    assert_eq!(mapped.data(), Some(1));
}

#[test]
fn mapped_resource_with_unit_error_type() {
    let transform = SelectTransform::new(|s: &String| s.len());
    let mapped = MappedQueryResource::<String, usize, ()>::new(
        Some(Arc::new("hello".to_string())),
        transform,
    );
    assert_eq!(mapped.data(), Some(5));
}

#[test]
fn mapped_resource_identity_transform() {
    let transform = SelectTransform::new(|x: &i32| *x);
    let mut mapped: MappedQueryResource<i32, i32, ()> =
        MappedQueryResource::new(Some(Arc::new(42)), transform);
    assert_eq!(mapped.data(), Some(42));

    mapped.update_source(Some(Arc::new(99)));
    assert_eq!(mapped.data(), Some(99));
}

#[test]
fn mapped_resource_complex_projection() {
    #[derive(Clone)]
    #[allow(dead_code)]
    struct User {
        name: String,
        age: u32,
        active: bool,
    }

    let active_names = SelectTransform::new(|users: &Vec<User>| {
        users
            .iter()
            .filter(|u| u.active)
            .map(|u| u.name.clone())
            .collect::<Vec<_>>()
    });

    let users = vec![
        User {
            name: "Alice".into(),
            age: 30,
            active: true,
        },
        User {
            name: "Bob".into(),
            age: 25,
            active: false,
        },
        User {
            name: "Carol".into(),
            age: 35,
            active: true,
        },
    ];

    let mapped: MappedQueryResource<Vec<User>, Vec<String>, ()> =
        MappedQueryResource::new(Some(Arc::new(users)), active_names);
    assert_eq!(
        mapped.data(),
        Some(vec!["Alice".to_string(), "Carol".to_string()])
    );
}
