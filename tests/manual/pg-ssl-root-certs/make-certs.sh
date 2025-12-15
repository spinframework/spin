# Create a directory for certificates
mkdir -p postgres-ssl
cd postgres-ssl


# 1. Generate CA private key
openssl genrsa -out ca.key 4096

# 2. Generate CA certificate (self-signed, valid for 10 years)
openssl req -new -x509 -key ca.key -out ca.crt -days 3650 \
  -subj "/C=US/ST=California/L=San Francisco/O=MyOrganization/OU=Certificate Authority/CN=MyOrganization Root CA"

# 3. Generate server private key
openssl genrsa -out server.key 2048

# 4. Generate server certificate signing request (CSR)
openssl req -new -key server.key -out server.csr \
  -subj "/C=US/ST=California/L=San Francisco/O=MyOrganization/OU=Development/CN=localhost"

# 5. Sign the server certificate with the CA
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -out server.crt -days 365

# 6. Set proper permissions
chmod 600 server.key ca.key
chmod 644 server.crt ca.crt

# 7. Clean up the CSR (optional)
rm server.csr

cd ..

psql -d postgres -c "ALTER SYSTEM SET ssl = 'on';" \
  -c "ALTER SYSTEM SET ssl_cert_file = '${PWD}/postgres-ssl/server.crt';" \
  -c "ALTER SYSTEM SET ssl_key_file = '${PWD}/postgres-ssl/server.key';" \
  -c "ALTER SYSTEM SET ssl_ca_file = '${PWD}/postgres-ssl/ca.crt';" \
  -c "SELECT pg_reload_conf();"

echo "consider running 'brew services restart postgresql' or 'sudo systemctl restart postgresql' to ensure changes take effect."