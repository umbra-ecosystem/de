use std::borrow::Cow;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOr<T: From<String>> {
    String(String),
    Value(T),
}

impl<T: From<String>> StringOr<T> {
    pub fn as_value(&self) -> Cow<T>
    where
        T: Clone,
    {
        match self {
            StringOr::String(s) => Cow::Owned(T::from(s.clone())),
            StringOr::Value(v) => Cow::Borrowed(v),
        }
    }

    pub fn into_inner(self) -> T {
        match self {
            StringOr::String(s) => T::from(s),
            StringOr::Value(v) => v,
        }
    }

    pub fn clone_value(&self) -> T
    where
        T: Clone,
    {
        match self {
            StringOr::String(s) => T::from(s.clone()),
            StringOr::Value(v) => v.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn as_slice(&self) -> &[T] {
        match self {
            OneOrMany::One(item) => std::slice::from_ref(item),
            OneOrMany::Many(items) => items.as_slice(),
        }
    }
}

impl<T: From<String>> From<String> for OneOrMany<T> {
    fn from(value: String) -> Self {
        OneOrMany::One(T::from(value))
    }
}
