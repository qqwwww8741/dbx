/** Property grammars used by Microsoft JDBC 8.4+ and Teradata JDBC. */

const SECRET_CONNECTION_PROPERTY = /^(?:password|pwd|pass|passcode|passphrase|token|secret|key|apikey|api_key|accessToken|access_token|logdata|new_password|ssltruststore_password|sslpassword|oauth_client_secret|client_secret|clientKeyPassword|keyStoreSecret|trustStorePassword)$/i;

export function isSecretConnectionProperty(key: string): boolean {
  const rawKey = key.trim();
  try {
    // URL import decodes property names before interpreting credentials.
    return SECRET_CONNECTION_PROPERTY.test(decodeURIComponent(rawKey).trim());
  } catch {
    return SECRET_CONNECTION_PROPERTY.test(rawKey);
  }
}

/** Preserve the JDBC grammar; callers decide whether to decode legacy unquoted values. */

/** mssql:// query strings can contain bare flags; JDBC property lists cannot. */

/** DBX also accepts &/; separated form parameters; never split inside quoted values. */
