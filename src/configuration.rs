use crate::authenticated_user::SharedAuthenticatedUsersHashMap;
use crate::authentication_middleware::SharedRequestData;
#[cfg(feature = "authentication-oidc")]
use crate::authentication_oidc::{OidcConfiguration, SharedOidcVerificationDataHashMap};
#[cfg(feature = "oidc-auth-ldap")]
use crate::authentication_url::AUTH_ROUTE;
#[cfg(feature = "ldap-common")]
use crate::ldap_common::LdapCommonConfiguration;
#[cfg(any(feature = "ldap-auth", feature = "authentication-oidc"))]
use crate::login_user_trait::Login;
use crate::mail_configuration::SendEMailConfiguration;
use crate::rsa_functions::{RsaKeys, RsaPrivateKeyPassword};
use crate::secret_functions::SharedSecretData;
#[cfg(feature = "authentication-oidc")]
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    reqwest::async_http_client,
    ClientId, ClientSecret, IssuerUrl, RedirectUrl,
};
use openssl::ssl::{SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod, SslOptions};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// valid secure cipers for TLS1v2 and TLS 1v3
const CIPHER_LIST: &str = concat!(
    "TLS_AES_128_GCM_SHA256:",
    "TLS_AES_256_GCM_SHA384:",
    "TLS_CHACHA20_POLY1305_SHA256:",
    "ECDHE-ECDSA-AES128-GCM-SHA256:",
    "ECDHE-RSA-AES128-GCM-SHA256:",
    "ECDHE-ECDSA-AES256-GCM-SHA384:",
    "ECDHE-RSA-AES256-GCM-SHA384:",
    "ECDHE-ECDSA-CHACHA20-POLY1305:",
    "ECDHE-RSA-CHACHA20-POLY1305:",
    "DHE-RSA-AES128-GCM-SHA256:",
    "DHE-RSA-AES256-GCM-SHA384"
);

/// Holds the deserialized entries of the json file
/// that is passed to the program
#[derive(Clone, Deserialize, Debug)]
pub struct ConfigurationFile {
    pub web_bind_address: String,
    pub ssl_private_key_file: String,
    pub ssl_certificate_chain_file: String,
    pub rsa_private_key_file: String,
    pub rsa_public_key_file: String,
    pub secret_directory: String,
    pub email_configuration: SendEMailConfiguration,
    pub admin_accounts: Vec<String>,
    pub max_authrequest_age_seconds: i64,
    pub max_cookie_age_seconds: i64,
    pub fqdn: String,
    #[cfg(feature = "ldap-common")]
    pub ldap_common_configuration: LdapCommonConfiguration,
    #[cfg(feature = "oidc-auth-ldap")]
    pub oidc_configuration: OidcConfiguration,
    pub login_hint: String,
    pub mail_hint: Option<String>,
    pub imprint: Imprint,
}

impl ConfigurationFile {
    /// get the domain part of the stored fqdn
    /// which contains the <domain>:<port>
    pub fn get_domain(&self) -> String {
        match self.fqdn.split_once(':') {
            Some((domain, _)) => String::from(domain),
            None => self.fqdn.clone(),
        }
    }
}

/// Loads a json file and deserializes it into an
/// instance of ConfigurationFile
impl ConfigurationFile {
    /// Read the web service configuration from a json file.
    pub fn read_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        // Open the file in read-only mode with buffer.
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        // Read the JSON contents of the file as an instance of `ConfigurationFile`.
        let mut parsed_config: ConfigurationFile = serde_json::from_reader(reader)?;
        // check if tke rsa key files exists because they are loaded later on,
        // when the password is entered by the administator.
        // The server must not start if such a key component is missing.
        if !Path::new(&parsed_config.rsa_private_key_file).exists() {
            panic!(
                "rsa private key file {} does not exist!",
                &parsed_config.rsa_private_key_file
            );
        }
        if !Path::new(&parsed_config.rsa_public_key_file).exists() {
            panic!(
                "rsa public key file {} does not exist!",
                &parsed_config.rsa_public_key_file
            );
        }
        #[cfg(feature = "ldap-auth")]
        parsed_config
            .ldap_common_configuration
            .build_valid_user_regex()?;
        #[cfg(feature = "oidc-auth-ldap")]
        parsed_config.oidc_configuration.build_valid_user_regex()?;
        Ok(parsed_config)
    }
}

/// Holds the complete configuration information
/// that is passed to the HttpServer
#[derive(Clone)]
pub struct ApplicationConfiguration {
    pub configuration_file: ConfigurationFile,
    /// password for rsa private key
    pub rsa_password: Arc<RwLock<RsaPrivateKeyPassword>>,
    // RSA keys
    pub rsa_keys: Arc<RwLock<RsaKeys>>,
    // SharedSecret (context for creating uuids)
    pub shared_secret: Arc<RwLock<SharedSecretData>>,
    /// stores authenticated users
    pub shared_authenticated_users: Arc<RwLock<SharedAuthenticatedUsersHashMap>>,
    /// stores every incoming resource request
    pub shared_request_data: Arc<RwLock<SharedRequestData>>,
    /// stores the optional oidc cliet configuration
    #[cfg(feature = "oidc-auth-ldap")]
    pub oidc_client: Arc<CoreClient>,
    /// stores the optional oidc verification data
    #[cfg(feature = "oidc-auth-ldap")]
    pub shared_oidc_verification_data: Arc<RwLock<SharedOidcVerificationDataHashMap>>,
}

/// Build a new instance of ApplicationConfiguration
impl ApplicationConfiguration {
    /// Reads the configuration file
    ///
    /// # Arguments
    ///
    /// - configuration_file_path: valid `path` of the file to read
    ///
    /// # Panics
    ///
    /// If the file is not to be found or can not be read, the function will panic.
    ///
    /// # Returns
    ///
    /// - `ApplicationConfiguration`
    pub async fn read_from_file<P: AsRef<Path>>(configuration_file_path: P) -> ApplicationConfiguration {
        let config_file = ConfigurationFile::read_from_file(configuration_file_path)
            .expect("Cannot load the json configuration file!");
        #[cfg(feature = "authentication-oidc")]
        let provider_metadata = CoreProviderMetadata::discover_async(
            IssuerUrl::new(config_file.oidc_configuration.provider_metadata_url.clone())
                .expect("Cannot build provider metadata url!"),
                async_http_client,
        ).await
        .expect("Cannot load oidc provider metadata");
        ApplicationConfiguration {
            configuration_file: config_file.clone(),
            rsa_password: Arc::new(RwLock::new(RsaPrivateKeyPassword {
                rsa_private_key_password: None,
            })),
            rsa_keys: Arc::new(RwLock::new(RsaKeys::new())),
            shared_secret: Arc::new(RwLock::new(SharedSecretData::new())),
            shared_authenticated_users: Arc::new(RwLock::new(
                SharedAuthenticatedUsersHashMap::new(config_file.admin_accounts),
            )),
            shared_request_data: Arc::new(RwLock::new(SharedRequestData::new())),
            #[cfg(feature = "authentication-oidc")]
            oidc_client: Arc::new(
                CoreClient::from_provider_metadata(
                    provider_metadata,
                    ClientId::new(config_file.oidc_configuration.client_id),
                    Some(ClientSecret::new(
                        config_file.oidc_configuration.client_secret,
                    )),
                )
                // Set the URL the user will be redirected to after the authorization process.
                .set_redirect_uri(
                    RedirectUrl::new(format!(
                        "https://{}/authentication{}",
                        &config_file.fqdn, AUTH_ROUTE
                    ))
                    .expect("Invalid redirect URL"),
                ),
            ),
            #[cfg(feature = "oidc-auth-ldap")]
            shared_oidc_verification_data: Arc::new(RwLock::new(
                SharedOidcVerificationDataHashMap::new(),
            )),
        }
    }

    /// Reads the RSA key files which are referenced in the configuration file
    ///
    /// If the files are not to be found or cannot be read, the function will
    /// return a boxed error.
    ///
    /// # Returns
    ///
    /// - `Result<(), Box<dyn Error>>`
    pub fn load_rsa_keys(&self) -> Result<(), Box<dyn Error>> {
        if let Some(rsa_private_key_password) =
            &self.rsa_password.read().unwrap().rsa_private_key_password
        {
            let mut rsa_write_lock = self.rsa_keys.write().unwrap();
            rsa_write_lock.read_from_files(
                &self.configuration_file.rsa_private_key_file,
                &self.configuration_file.rsa_public_key_file,
                rsa_private_key_password,
            )
        } else {
            const RSA_PASSWORD_NOT_SET: &str = "Password not set, inform system administrator";
            let boxed_error = Box::<dyn Error + Send + Sync>::from(RSA_PASSWORD_NOT_SET);
            Err(boxed_error)
        }
    }

    /// Clears the passsword for the RSA key files which are referenced in the configuration file
    ///
    /// If the files are not to be found or cannot be read, the function will
    /// return a boxed error.
    ///
    /// # Returns
    ///
    /// - `Result<(), Box<dyn Error>>`
    pub fn clear_rsa_password(&self) -> Result<(), Box<dyn Error>> {
        self.rsa_password.write().unwrap().rsa_private_key_password = None;
        Ok(())
    }

    /// Build the `SslAcceptorBuilder` for HTTPS connections
    ///
    /// # Returns
    ///
    /// - `SslAcceptorBuilder`
    pub fn get_ssl_acceptor_builder(&self) -> SslAcceptorBuilder {
        let mut ssl_acceptor_builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        ssl_acceptor_builder
            .set_private_key_file(
                &self.configuration_file.ssl_private_key_file,
                SslFiletype::PEM,
            )
            .unwrap();
        ssl_acceptor_builder
            .set_certificate_chain_file(&self.configuration_file.ssl_certificate_chain_file)
            .unwrap();
        ssl_acceptor_builder
            .set_options(SslOptions::NO_SSLV2 | SslOptions::NO_SSLV3 | SslOptions::NO_TLSV1_1);
        ssl_acceptor_builder.set_cipher_list(CIPHER_LIST).unwrap();
        ssl_acceptor_builder
    }
}

/// Valid targets for opening the link to the imprint page
#[derive(Clone, Deserialize, Serialize, Debug)]
pub enum Target {
    #[serde(rename = "_blank")]
    Blank,
    #[serde(rename = "_self")]
    _Self,
    #[serde(rename = "_parent")]
    Parent,
    #[serde(rename = "_top")]
    Top,
}

/// Link information for the imprint page
#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Imprint {
    pub href: String,
    pub target: Target,
}
