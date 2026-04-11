use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    sub: Option<String>,
    user_id: Option<String>,
    organization_id: Option<String>,
    exp: usize,
}

pub struct JwtValidator {
    decoding_key: DecodingKey,
    validation: Validation,
}

impl JwtValidator {
    pub fn new(config: &AppConfig) -> AppResult<Self> {
        let decoding_key = DecodingKey::from_rsa_pem(config.jwt_public_key_pem.as_bytes())
            .map_err(|err| AppError::Jwt(err.to_string()))?;

        let validation = Validation::new(Algorithm::RS256);

        Ok(Self {
            decoding_key,
            validation,
        })
    }

    pub fn validate_bearer(&self, bearer: &str) -> AppResult<AuthContext> {
        let token = bearer
            .strip_prefix("Bearer ")
            .or_else(|| bearer.strip_prefix("bearer "))
            .ok_or_else(|| AppError::Jwt("missing Bearer token prefix".to_string()))?;

        let token_data = decode::<JwtClaims>(token, &self.decoding_key, &self.validation)
            .map_err(|err| AppError::Jwt(err.to_string()))?;

        let claims = token_data.claims;
        let _exp = claims.exp;

        let user_id = claims
            .user_id
            .or(claims.sub)
            .ok_or_else(|| AppError::Jwt("missing user_id claim".to_string()))?;

        let organization_id = claims
            .organization_id
            .ok_or_else(|| AppError::Jwt("missing organization_id claim".to_string()))?;

        let user_id = Uuid::parse_str(&user_id)
            .map_err(|_| AppError::InvalidUuid("user_id claim".to_string()))?;
        let organization_id = Uuid::parse_str(&organization_id)
            .map_err(|_| AppError::InvalidUuid("organization_id claim".to_string()))?;

        Ok(AuthContext {
            user_id,
            organization_id,
        })
    }
}
