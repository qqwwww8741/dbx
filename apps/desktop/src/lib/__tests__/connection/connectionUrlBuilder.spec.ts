import { describe, expect, it } from "vitest";
import { buildConnectionUrlCopy, connectionUrlCopyFormats, type ConnectionUrlCopyConfig } from "@/lib/connection/connectionUrlBuilder";
import { parseConnectionUrl } from "@/lib/connection/connectionUrl";
const config: ConnectionUrlCopyConfig = { db_type: "mysql", host: "2001:db8::1", port: 3307, username: "user @你好", password: "p:@/?#'", database: "订单", url_params: "connect_timeout=10", ssl: true };
describe("MySQL connection URLs", () => {
  it.each(["urlWithPassword", "jdbcUrlWithCredentials"] as const)("round-trips credentials, Unicode, IPv6 and TLS in %s", (format) => {
    const parsed = parseConnectionUrl(buildConnectionUrlCopy(config, format)!);
    expect(parsed).toMatchObject({ dbType: "mysql", driverProfile: "mysql", host: config.host, port: config.port, username: config.username, password: config.password, database: config.database, ssl: true });
    expect(parsed.urlParams).toContain("connect_timeout=10");
  });
  it.each(["url", "jdbcUrl"] as const)("redacts credentials and query secrets in %s", (format) => {
    const url = buildConnectionUrlCopy({ ...config, url_params: "token=private-token&sslpassword=private-key-pass&connect_timeout=10" }, format)!;
    expect(url).not.toContain("private-token");
    expect(url).not.toContain("private-key-pass");
    expect(url).not.toContain(encodeURIComponent(config.password));
    expect(url).toContain("connect_timeout=10");
  });
  it("omits password formats without stored secrets", () => {
    expect(connectionUrlCopyFormats({ ...config, password: "", url_params: "" })).toEqual(["url", "jdbcUrl", "hostPort"]);
    expect(connectionUrlCopyFormats(undefined)).toEqual([]);
    expect(buildConnectionUrlCopy(config, "hostPort")).toBe("[2001:db8::1]:3307");
  });
  it.each(["postgres://user@localhost/db", "redis://localhost:6379", "jdbc:oracle:thin:@localhost:1521/orcl", "sqlite:///tmp/test.db"])("rejects removed engines: %s", (url) => {
    expect(() => parseConnectionUrl(url)).toThrow();
  });
  it("accepts the MySQL default port and JDBC query credentials", () => {
    expect(parseConnectionUrl("jdbc:mysql://localhost/app?user=a&password=b&useSSL=true")).toMatchObject({ host: "localhost", port: 3306, username: "a", password: "b", database: "app", ssl: true });
  });
});
