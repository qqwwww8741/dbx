# MySQL connection guide

Create a MySQL connection using a host, port (3306 by default), username, password, and optional default database. A standard `mysql://` URL can also populate the connection form. Other database schemes are rejected.

Use the TLS tab to configure certificate verification, CA certificates, and client certificates. Transport settings support SSH tunnels, proxies, and HTTP tunnels. Keep credentials out of shared connection URLs and use the password-saving preference when persisting connections.

The SQL editor, table browser, schema editor, data grid, import/export, and backup features all target MySQL. CLI and MCP use the same MySQL connection registry as the desktop and Web applications.
