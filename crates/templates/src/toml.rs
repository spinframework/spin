pub(crate) fn get_at<S: AsRef<str>>(value: toml::Value, path: &[S]) -> Option<toml::Value> {
    match path.split_first() {
        None => Some(value), // we are at the end of the path and we have a thing
        Some((first, rest)) => {
            match value.as_table() {
                None => None, // we need to key into it and we can't
                Some(t) => {
                    match t.get(first.as_ref()) {
                        None => None,                       // we tried to key into it and no match
                        Some(v) => get_at(v.clone(), rest), // we pathed into it! keep pathing
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn if_path_does_not_exist_then_get_at_is_none() {
        let document: toml::Value = toml::toml! {
            name = "test"

            [application.redis.trigger]
            address = "test-address"

            [[trigger.redis]]
            channel = "messages"
        }
        .into();

        assert!(get_at(document.clone(), &["name", "snort"]).is_none());
        assert!(get_at(document.clone(), &["snort", "fie"]).is_none());
        assert!(get_at(document.clone(), &["application", "snort"]).is_none());
        assert!(get_at(document.clone(), &["application", "redis", "snort"]).is_none());
        assert!(get_at(document.clone(), &["trigger", "redis", "snort"]).is_none());

        // We have not yet needed to define a behaviour for seeking into table arrays, but
        // presumably it will need some sort of disambiguation for array element.
        // For now, we assume that eithout disambiguation it will report no result.
        assert!(get_at(document.clone(), &["trigger", "redis", "channel"]).is_none());
    }

    #[test]
    fn if_path_does_exist_then_get_at_finds_it() {
        let document: toml::Value = toml::toml! {
            name = "test"

            [application.redis.trigger]
            address = "test-address"

            [[trigger.redis]]
            channel = "messages"
        }
        .into();

        assert!(
            get_at(document.clone(), &["name"])
                .expect("should find name")
                .is_str()
        );
        assert!(
            get_at(document.clone(), &["application", "redis"])
                .expect("should find application.redis")
                .is_table()
        );
        assert!(
            get_at(document.clone(), &["application", "redis", "trigger"])
                .expect("should find application.redis.trigger")
                .is_table()
        );
        assert!(
            get_at(document.clone(), &["trigger", "redis"])
                .expect("should find trigger.redis.channel")
                .is_array()
        );
    }
}
