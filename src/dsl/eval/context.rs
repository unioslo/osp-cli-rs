#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::output_model::compute_key_index;

    #[test]
    fn keeps_first_seen_key_order() {
        let rows = vec![
            json!({"uid": "oistes", "cn": "Oistein"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"mail": "o@uio.no", "uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let key_index = compute_key_index(&rows);
        assert_eq!(key_index.as_slice(), &["uid", "cn", "mail"]);
    }
}
