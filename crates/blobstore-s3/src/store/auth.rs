/// AWS S3 runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct S3KeyAuth {
    /// The access key for the AWS S3 account role.
    pub access_key: String,
    /// The secret key for authorization on the AWS S3 account.
    pub secret_key: String,
    /// The token for authorization on the AWS S3 account.
    pub token: Option<String>,
}

impl S3KeyAuth {
    pub fn new(access_key: String, secret_key: String, token: Option<String>) -> Self {
        Self {
            access_key,
            secret_key,
            token,
        }
    }
}

impl aws_credential_types::provider::ProvideCredentials for S3KeyAuth {
    fn provide_credentials<'a>(
        &'a self,
    ) -> aws_credential_types::provider::future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        aws_credential_types::provider::future::ProvideCredentials::ready(Ok(
            aws_credential_types::Credentials::new(
                self.access_key.clone(),
                self.secret_key.clone(),
                self.token.clone(),
                None, // Optional expiration time
                "spin_custom_s3_provider",
            ),
        ))
    }
}

/// AWS S3 authentication options
#[derive(Clone, Debug)]
pub enum S3AuthOptions {
    /// The account and key have been specified directly
    AccessKey(S3KeyAuth),
    /// Use environment variables
    Environmental,
}
