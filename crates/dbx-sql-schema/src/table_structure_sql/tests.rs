use super::*;
use crate::models::connection::DatabaseType;
use crate::types::PgPartitionKind;

fn column(name: &str) -> EditableStructureColumn {
    EditableStructureColumn {
        id: name.to_string(),
        name: name.to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        default_value: String::new(),
        comment: String::new(),
        is_primary_key: false,
        extra: None,
        original: None,
        original_position: None,
        marked_for_drop: false,
        character_set: String::new(),
        collation: String::new(),
    }
}

/// Existing column draft with optional primary-key membership change.
fn existing_pk_column(
    name: &str,
    data_type: &str,
    was_primary_key: bool,
    is_primary_key: bool,
) -> EditableStructureColumn {
    let mut col = column(name);
    col.data_type = data_type.to_string();
    col.is_nullable = false;
    col.is_primary_key = is_primary_key;
    col.original = Some(ColumnInfo {
        name: name.to_string(),
        data_type: data_type.to_string(),
        is_nullable: false,
        column_default: None,
        is_primary_key: was_primary_key,
        extra: None,
        comment: None,
        ..Default::default()
    });
    col
}

fn structure_change_options(
    database_type: DatabaseType,
    schema: Option<&str>,
    table_name: &str,
    columns: Vec<EditableStructureColumn>,
) -> TableStructureSqlOptions {
    TableStructureSqlOptions {
        database_type: Some(database_type),
        driver_profile: None,
        schema: schema.map(str::to_string),
        table_name: table_name.to_string(),
        columns,
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    }
}

#[test]
fn mysql_table_engine_change_generates_alter_table() {
    let mut options = structure_change_options(DatabaseType::Mysql, Some("dbx_test"), "remote_orders", Vec::new());
    options.mysql_engine = Some("FEDERATED".to_string());

    let result = build_table_structure_change_sql(options);

    assert!(result.warnings.is_empty());
    assert_eq!(result.statements, vec!["ALTER TABLE `remote_orders` ENGINE = FEDERATED;"]);
}

#[test]
fn mysql_create_table_includes_engine_before_comment() {
    let mut options = structure_change_options(DatabaseType::Mysql, Some("dbx_test"), "archive", vec![column("id")]);
    options.mysql_engine = Some("MyISAM".to_string());
    options.table_comment = Some("remote archive".to_string());

    let result = build_create_table_sql(options);

    assert!(result.warnings.is_empty());
    assert_eq!(
        result.statements[0],
        "CREATE TABLE `archive` (\n  `id` varchar(255)\n) ENGINE = MyISAM COMMENT = 'remote archive';"
    );
}

fn index(name: &str, columns: &[&str]) -> EditableStructureIndex {
    EditableStructureIndex {
        id: name.to_string(),
        name: name.to_string(),
        columns: columns.iter().map(|column| column.to_string()).collect(),
        is_unique: false,
        is_primary: false,
        filter: String::new(),
        index_type: String::new(),
        included_columns: Vec::new(),
        column_opclasses: Vec::new(),
        comment: String::new(),
        concurrently: false,
        original: None,
        marked_for_drop: false,
    }
}

fn existing_index(name: &str, columns: &[&str], is_unique: bool) -> EditableStructureIndex {
    let mut index = index(name, columns);
    index.is_unique = is_unique;
    index.original = Some(IndexInfo {
        name: name.to_string(),
        columns: columns.iter().map(|column| column.to_string()).collect(),
        is_unique,
        is_primary: false,
        filter: None,
        index_type: None,
        included_columns: None,
        comment: None,
        key_is_expression: Vec::new(),
        column_opclasses: vec![],
        constraint_backed: false,
    });
    index
}

fn index_change_options(
    database_type: DatabaseType,
    schema: Option<&str>,
    index: EditableStructureIndex,
) -> TableStructureSqlOptions {
    TableStructureSqlOptions {
        database_type: Some(database_type),
        driver_profile: None,
        schema: schema.map(str::to_string),
        table_name: "USERS".to_string(),
        columns: Vec::new(),
        indexes: vec![index],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    }
}

fn foreign_key(name: &str, column: &str, ref_table: &str, ref_column: &str) -> EditableStructureForeignKey {
    EditableStructureForeignKey {
        id: name.to_string(),
        name: name.to_string(),
        column: column.to_string(),
        ref_schema: String::new(),
        ref_table: ref_table.to_string(),
        ref_column: ref_column.to_string(),
        on_update: String::new(),
        on_delete: String::new(),
        original: None,
        marked_for_drop: false,
    }
}

fn trigger(name: &str, timing: &str, event: &str, statement: &str) -> EditableStructureTrigger {
    EditableStructureTrigger {
        id: name.to_string(),
        name: name.to_string(),
        timing: timing.to_string(),
        event: event.to_string(),
        statement: statement.to_string(),
        original: None,
        marked_for_drop: false,
    }
}

#[test]
fn builds_mysql_column_and_index_changes() {
    let mut renamed = column("display_name");
    renamed.data_type = "varchar(120)".to_string();
    renamed.is_nullable = false;
    renamed.default_value = "guest".to_string();
    renamed.comment = "Shown name".to_string();
    renamed.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(80)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: Some(String::new()),
        ..Default::default()
    });
    let mut email = column("email");
    email.is_nullable = false;
    let mut old_index = index("idx_old", &["name"]);
    old_index.marked_for_drop = true;
    old_index.original = Some(IndexInfo {
        name: "idx_old".to_string(),
        columns: vec!["name".to_string()],
        is_unique: false,
        is_primary: false,
        filter: None,
        index_type: None,
        included_columns: None,
        comment: None,
        key_is_expression: Vec::new(),
        column_opclasses: vec![],
        constraint_backed: false,
    });
    let mut email_index = index("uniq_users_email", &["email"]);
    email_index.is_unique = true;

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![renamed, email],
        indexes: vec![old_index, email_index],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` CHANGE COLUMN `name` `display_name` varchar(120) NOT NULL DEFAULT 'guest' COMMENT 'Shown name';",
            "ALTER TABLE `users` ADD COLUMN `email` varchar(255) NOT NULL;",
            "DROP INDEX `idx_old` ON `users`;",
            "CREATE UNIQUE INDEX `uniq_users_email` ON `users` (`email`);",
        ]
    );
}

#[test]
fn builds_mysql_unsigned_integer_column_with_length_before_attribute() {
    let mut score = column("score");
    score.data_type = "int unsigned(11)".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![score],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `users` ADD COLUMN `score` int(11) unsigned;"]);
}

#[test]
fn mysql_create_index_with_comment() {
    let mut col = column("name");
    col.data_type = "varchar(120)".to_string();
    let mut idx = index("idx_users_name", &["name"]);
    idx.comment = "Search index".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: vec![idx],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` ADD COLUMN `name` varchar(120);",
            "CREATE INDEX `idx_users_name` ON `users` (`name`) COMMENT 'Search index';",
        ]
    );
}

#[test]
fn mysql_create_unique_index_with_comment_and_btree() {
    let mut idx = index("uniq_users_email", &["email"]);
    idx.is_unique = true;
    idx.index_type = "BTREE".to_string();
    idx.comment = "Unique email index".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: Vec::new(),
        indexes: vec![idx],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE UNIQUE INDEX `uniq_users_email` USING BTREE ON `users` (`email`) COMMENT 'Unique email index';",]
    );
}

#[test]
fn mysql_create_functional_index_preserves_key_part_syntax() {
    let functional_key_part = "((case when (`STATUS` = _utf8mb4'online') then _utf8mb4'online' else NULL end))";
    let mut idx = index("test_UNIQUE", &["attr", "attr2", functional_key_part]);
    idx.is_unique = true;

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "test".to_string(),
        columns: Vec::new(),
        indexes: vec![idx],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![format!("CREATE UNIQUE INDEX `test_UNIQUE` ON `test` (`attr`, `attr2`, {functional_key_part});")]
    );
}

#[test]
fn mysql_add_timestamp_column_drops_invalid_precision() {
    let mut created_at = column("created_at");
    created_at.data_type = "timestamp(255)".to_string();
    created_at.default_value = "CURRENT_TIMESTAMP".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![created_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` ADD COLUMN `created_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP;"]
    );
}

#[test]
fn mysql_add_timestamp_column_preserves_valid_precision() {
    let mut created_at = column("created_at");
    created_at.data_type = "timestamp(3)".to_string();
    created_at.default_value = "CURRENT_TIMESTAMP(3)".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![created_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` ADD COLUMN `created_at` timestamp(3) NULL DEFAULT CURRENT_TIMESTAMP(3);"]
    );
}

#[test]
fn create_table_trims_table_name_whitespace_for_all_statements() {
    let mut id = column("id");
    id.data_type = "integer".to_string();
    let idx = index("idx_users_id", &["id"]);

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "  users  ".to_string(),
        columns: vec![id],
        indexes: vec![idx],
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE TABLE `users` (\n  `id` integer\n);", "CREATE INDEX `idx_users_id` ON `users` (`id`);",]
    );
}

#[test]
fn builds_mysql_column_reorder_statements() {
    let mut id = column("id");
    id.data_type = "int".to_string();
    id.is_nullable = false;
    id.is_primary_key = true;
    id.original_position = Some(0);
    id.original = Some(ColumnInfo {
        name: "id".to_string(),
        data_type: "int".to_string(),
        is_nullable: false,
        column_default: None,
        is_primary_key: true,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut email = column("email");
    email.original_position = Some(2);
    email.original = Some(ColumnInfo {
        name: "email".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut name = column("display_name");
    name.id = "name".to_string();
    name.data_type = "varchar(120)".to_string();
    name.original_position = Some(1);
    name.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(80)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![id, email, name],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` CHANGE COLUMN `name` `display_name` varchar(120) AFTER `email`;"]
    );
}

#[test]
fn mysql_add_column_before_existing_column_does_not_reorder_shifted_column() {
    let mut deleted = column("deleted");
    deleted.original_position = Some(0);
    deleted.original = Some(ColumnInfo {
        name: "deleted".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let new_column = column("sss");

    let mut tenant_id = column("tenant_id");
    tenant_id.data_type = "bigint".to_string();
    tenant_id.is_nullable = false;
    tenant_id.default_value = "0".to_string();
    tenant_id.comment = "tenant id".to_string();
    tenant_id.original_position = Some(1);
    tenant_id.original = Some(ColumnInfo {
        name: "tenant_id".to_string(),
        data_type: "bigint".to_string(),
        is_nullable: false,
        column_default: Some("0".to_string()),
        is_primary_key: false,
        extra: None,
        comment: Some("tenant id".to_string()),
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "infra_api_error_log".to_string(),
        columns: vec![deleted, new_column, tenant_id],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `infra_api_error_log` ADD COLUMN `sss` varchar(255) AFTER `deleted`;"]
    );
}

#[test]
fn mysql_existing_column_reorder_does_not_reorder_columns_shifted_by_prior_move() {
    let mut id = column("id");
    id.original_position = Some(0);
    id.original = Some(ColumnInfo {
        name: "id".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut name = column("name");
    name.original_position = Some(1);
    name.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut email = column("email");
    email.original_position = Some(2);
    email.original = Some(ColumnInfo {
        name: "email".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![id, email, name],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `users` MODIFY COLUMN `name` varchar(255) AFTER `email`;"]);
}

#[test]
fn mysql_moving_first_column_to_end_uses_single_reorder_statement() {
    let mut col_0 = column("col_0");
    col_0.data_type = "int(11)".to_string();
    col_0.is_nullable = false;
    col_0.original_position = Some(0);
    col_0.original = Some(ColumnInfo {
        name: "col_0".to_string(),
        data_type: "int(11)".to_string(),
        is_nullable: false,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut col_1 = column("col_1");
    col_1.original_position = Some(1);
    col_1.original = Some(ColumnInfo {
        name: "col_1".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut col_2 = column("col_2");
    col_2.original_position = Some(2);
    col_2.original = Some(ColumnInfo {
        name: "col_2".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let mut col_3 = column("col_3");
    col_3.original_position = Some(3);
    col_3.original = Some(ColumnInfo {
        name: "col_3".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col_1, col_2, col_3, col_0],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `users` MODIFY COLUMN `col_0` int(11) NOT NULL AFTER `col_3`;"]);
}

#[test]
fn builds_mysql_alter_table_change_primary_key() {
    let mut old_pk = existing_pk_column("id", "int", true, false);
    old_pk.id = "old_id".to_string();
    let mut new_pk = existing_pk_column("uuid", "varchar(36)", false, true);
    new_pk.id = "new_uuid".to_string();

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "users",
        vec![old_pk, new_pk],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` DROP PRIMARY KEY;", "ALTER TABLE `users` ADD PRIMARY KEY (`uuid`);",]
    );
}

#[test]
fn mysql_coalesces_auto_increment_primary_key_migration() {
    let mut old_pk = existing_pk_column("campaign_rel_id", "bigint(20)", true, false);
    old_pk.id = "old_campaign_rel_id".to_string();

    let mut id = existing_pk_column("id", "bigint(20)", false, true);
    id.id = "new_id".to_string();
    id.comment = "自增主键".to_string();
    id.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

    let mut options = structure_change_options(DatabaseType::Mysql, None, "tbl_gy_campaign_rel", vec![old_pk, id]);
    options.indexes = vec![existing_index("campaign_rel_id_UNIQUE", &["campaign_rel_id"], true)];

    let result = build_table_structure_change_sql(options);

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `tbl_gy_campaign_rel` DROP PRIMARY KEY, MODIFY COLUMN `id` bigint(20) NOT NULL AUTO_INCREMENT COMMENT '自增主键', ADD PRIMARY KEY (`id`);"
        ]
    );
}

#[test]
fn mysql_coalesces_new_auto_increment_primary_key_column() {
    let mut old_pk = existing_pk_column("legacy_id", "bigint", true, false);
    old_pk.id = "old_legacy_id".to_string();

    let mut id = column("id");
    id.data_type = "bigint".to_string();
    id.is_nullable = false;
    id.is_primary_key = true;
    id.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "users",
        vec![old_pk, id],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` DROP PRIMARY KEY, ADD COLUMN `id` bigint NOT NULL AUTO_INCREMENT, ADD PRIMARY KEY (`id`);"
        ]
    );
}

#[test]
fn mysql_stable_primary_key_auto_increment_changes_keep_column_only_alter() {
    let mut enable = existing_pk_column("id", "bigint", true, true);
    enable.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

    let enabled =
        build_table_structure_change_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![enable]));
    assert_eq!(enabled.warnings, Vec::<String>::new());
    assert_eq!(enabled.statements, vec!["ALTER TABLE `users` MODIFY COLUMN `id` bigint NOT NULL AUTO_INCREMENT;"]);

    let mut disable = existing_pk_column("id", "bigint", true, true);
    disable.extra = Some(ColumnExtra::default());
    disable.original.as_mut().unwrap().extra = Some("auto_increment".to_string());

    let disabled =
        build_table_structure_change_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![disable]));
    assert_eq!(disabled.warnings, Vec::<String>::new());
    assert_eq!(disabled.statements, vec!["ALTER TABLE `users` MODIFY COLUMN `id` bigint NOT NULL;"]);
}

#[test]
fn mysql_coalesces_migration_away_from_existing_auto_increment_primary_key() {
    let mut old_pk = existing_pk_column("id", "bigint", true, false);
    old_pk.id = "old_id".to_string();
    old_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    old_pk.original.as_mut().unwrap().extra = Some("auto_increment".to_string());

    let mut new_pk = existing_pk_column("external_id", "varchar(64)", false, true);
    new_pk.id = "new_external_id".to_string();

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "users",
        vec![old_pk, new_pk],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    // The old key column keeps its AUTO_INCREMENT checkbox in the draft, but MySQL refuses an
    // auto column that no longer leads a key, so the flag is cleared in the same statement.
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` DROP PRIMARY KEY, MODIFY COLUMN `id` bigint NOT NULL, ADD PRIMARY KEY (`external_id`);"
        ]
    );
}

/// Regression for #7973: swapping the primary key onto another column left the previous
/// AUTO_INCREMENT key column untouched, so the coalesced ALTER failed with
/// `ERROR 1075 Incorrect table definition; there can be only one auto column ...`.
#[test]
fn mysql_clears_auto_increment_on_column_replaced_by_new_auto_increment_primary_key() {
    let mut old_pk = existing_pk_column("ID", "bigint unsigned", true, false);
    old_pk.id = "old_id".to_string();
    old_pk.comment = "自增ID".to_string();
    old_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    let old_original = old_pk.original.as_mut().unwrap();
    old_original.extra = Some("auto_increment".to_string());
    old_original.comment = Some("自增ID".to_string());

    let mut new_pk = existing_pk_column("ProjectID", "bigint", false, true);
    new_pk.id = "project_id".to_string();
    new_pk.comment = "对应project表的主键ID".to_string();
    new_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    new_pk.original.as_mut().unwrap().comment = Some("对应project表的主键ID".to_string());

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "issue7973_repro",
        vec![old_pk, new_pk],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `issue7973_repro` DROP PRIMARY KEY, MODIFY COLUMN `ID` bigint unsigned NOT NULL COMMENT '自增ID', MODIFY COLUMN `ProjectID` bigint NOT NULL AUTO_INCREMENT COMMENT '对应project表的主键ID', ADD PRIMARY KEY (`ProjectID`);"
        ]
    );
}

/// A surviving secondary index still keys the column, so AUTO_INCREMENT stays legal there
/// and must not be stripped just because the primary key moved elsewhere.
#[test]
fn mysql_keeps_auto_increment_when_a_kept_index_still_leads_with_the_column() {
    let mut old_pk = existing_pk_column("id", "bigint", true, false);
    old_pk.id = "old_id".to_string();
    old_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    old_pk.original.as_mut().unwrap().extra = Some("auto_increment".to_string());

    let mut new_pk = existing_pk_column("external_id", "varchar(64)", false, true);
    new_pk.id = "new_external_id".to_string();

    let mut options = structure_change_options(DatabaseType::Mysql, None, "users", vec![old_pk, new_pk]);
    options.indexes = vec![existing_index("idx_users_id", &["id"], false)];

    let result = build_table_structure_change_sql(options);

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `users` DROP PRIMARY KEY, ADD PRIMARY KEY (`external_id`);"]);
}

/// An index the same draft edits is rebuilt as DROP + CREATE *after* the column DDL, so it
/// cannot excuse the AUTO_INCREMENT flag: the DROP INDEX would hit ERROR 1075 itself.
#[test]
fn mysql_clears_auto_increment_when_the_only_covering_index_is_rebuilt_by_the_same_draft() {
    let mut old_pk = existing_pk_column("id", "bigint", true, false);
    old_pk.id = "old_id".to_string();
    old_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    old_pk.original.as_mut().unwrap().extra = Some("auto_increment".to_string());

    let mut new_pk = existing_pk_column("external_id", "varchar(64)", false, true);
    new_pk.id = "new_external_id".to_string();

    let mut rebuilt_index = existing_index("idx_users_id", &["id"], false);
    rebuilt_index.is_unique = true;

    let mut options = structure_change_options(DatabaseType::Mysql, None, "users", vec![old_pk, new_pk]);
    options.indexes = vec![rebuilt_index];

    let result = build_table_structure_change_sql(options);

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` DROP PRIMARY KEY, MODIFY COLUMN `id` bigint NOT NULL, ADD PRIMARY KEY (`external_id`);",
            "DROP INDEX `idx_users_id` ON `users`;",
            "CREATE UNIQUE INDEX `idx_users_id` ON `users` (`id`);",
        ]
    );
}

/// The covering index is matched by persisted names on both sides, so a draft that swaps two
/// column names cannot credit one column's index to the other.
#[test]
fn mysql_index_cover_does_not_follow_a_column_name_swap() {
    let mut old_pk = existing_pk_column("id", "bigint", true, false);
    old_pk.id = "old_id".to_string();
    old_pk.name = "legacy_id".to_string();
    old_pk.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });
    old_pk.original.as_mut().unwrap().extra = Some("auto_increment".to_string());

    // Takes over the name the surviving index was built on, but not the index itself.
    let mut renamed = existing_pk_column("tenant_id", "bigint", false, false);
    renamed.id = "tenant".to_string();
    renamed.name = "id".to_string();

    let mut new_pk = existing_pk_column("external_id", "varchar(64)", false, true);
    new_pk.id = "new_external_id".to_string();

    let mut options = structure_change_options(DatabaseType::Mysql, None, "users", vec![old_pk, renamed, new_pk]);
    options.indexes = vec![existing_index("idx_users_tenant_id", &["tenant_id"], false)];

    let result = build_table_structure_change_sql(options);

    assert_eq!(result.warnings, Vec::<String>::new());
    // Only the second statement is what this test is about: the auto column lost AUTO_INCREMENT
    // even though `idx_users_tenant_id` leads with a column *named* `id` in the draft. The
    // separate rename statement, and the order the two renames run in, are pre-existing
    // name-swap behavior unrelated to this fix.
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` CHANGE COLUMN `tenant_id` `id` bigint NOT NULL;",
            "ALTER TABLE `users` DROP PRIMARY KEY, CHANGE COLUMN `id` `legacy_id` bigint NOT NULL, ADD PRIMARY KEY (`external_id`);",
        ]
    );
}

#[test]
fn mysql_coalesces_renamed_auto_increment_primary_key_column() {
    let mut old_pk = existing_pk_column("campaign_rel_id", "bigint", true, false);
    old_pk.id = "old_campaign_rel_id".to_string();

    let mut id = existing_pk_column("id", "bigint", false, true);
    id.id = "new_id".to_string();
    id.original.as_mut().unwrap().name = "legacy_id".to_string();
    id.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "users",
        vec![old_pk, id],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` DROP PRIMARY KEY, CHANGE COLUMN `legacy_id` `id` bigint NOT NULL AUTO_INCREMENT, ADD PRIMARY KEY (`id`);"
        ]
    );
}

#[test]
fn mysql_coalesces_composite_primary_key_around_auto_increment_column() {
    for (auto_first, has_supporting_index, expected_primary_key) in
        [(true, false, "`id`, `tenant_id`"), (false, false, "`tenant_id`, `id`"), (false, true, "`tenant_id`, `id`")]
    {
        let mut old_pk = existing_pk_column("legacy_id", "bigint", true, false);
        old_pk.id = "old_legacy_id".to_string();

        let mut id = existing_pk_column("id", "bigint", false, true);
        id.id = "new_id".to_string();
        id.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

        let mut tenant_id = existing_pk_column("tenant_id", "bigint", false, true);
        tenant_id.id = "new_tenant_id".to_string();

        let columns = if auto_first { vec![old_pk, id, tenant_id] } else { vec![old_pk, tenant_id, id] };
        let mut options = structure_change_options(DatabaseType::Mysql, None, "users", columns);
        if has_supporting_index {
            options.indexes = vec![existing_index("uniq_users_id", &["id"], true)];
        }

        let result = build_table_structure_change_sql(options);

        assert_eq!(result.warnings, Vec::<String>::new());
        assert_eq!(result.statements.len(), 1);
        assert_eq!(
            result.statements[0],
            format!(
                "ALTER TABLE `users` DROP PRIMARY KEY, MODIFY COLUMN `id` bigint NOT NULL AUTO_INCREMENT, ADD PRIMARY KEY ({expected_primary_key});"
            )
        );
    }
}

#[test]
fn mysql_create_table_with_auto_increment() {
    let mut col = column("id");
    col.data_type = "int".to_string();
    col.is_nullable = false;
    col.is_primary_key = true;
    col.extra = Some(ColumnExtra { auto_increment: Some(true), ..Default::default() });

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements.len(), 1);
    assert!(result.statements[0].contains("AUTO_INCREMENT"));
}

#[test]
fn mysql_create_table_keeps_column_charset_collation_and_comment() {
    let mut name = column("name");
    name.data_type = "varchar(255)".to_string();
    name.character_set = "gbk".to_string();
    name.collation = "gbk_bin".to_string();
    name.comment = "测试".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![name],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: Some("User accounts".to_string()),
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "CREATE TABLE `users` (\n  `name` varchar(255) CHARACTER SET `gbk` COLLATE `gbk_bin` COMMENT '测试'\n) COMMENT = 'User accounts';"
        ]
    );
}

#[test]
fn mysql_create_table_with_on_update_current_timestamp() {
    let mut col = column("updated_at");
    col.data_type = "timestamp".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("ON UPDATE CURRENT_TIMESTAMP"));
}

#[test]
fn mysql_create_table_carries_temporal_precision_into_current_timestamp_clauses() {
    let mut col = column("updated_at");
    col.data_type = "datetime(3)".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });

    let result = build_create_table_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![col]));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE TABLE `users` (\n  `updated_at` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3)\n);"]
    );
}

#[test]
fn mysql_create_table_keeps_explicit_zero_temporal_precision_in_current_timestamp_clauses() {
    let mut col = column("updated_at");
    col.data_type = "datetime(0)".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });

    let result = build_create_table_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![col]));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE TABLE `users` (\n  `updated_at` datetime(0) NOT NULL DEFAULT CURRENT_TIMESTAMP(0) ON UPDATE CURRENT_TIMESTAMP(0)\n);"]
    );
}

#[test]
fn mysql_single_column_alter_carries_temporal_precision_into_current_timestamp_clauses() {
    let mut col = column("updated_at");
    col.data_type = "datetime(3)".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });
    col.original = Some(ColumnInfo {
        name: "updated_at".to_string(),
        data_type: "datetime".to_string(),
        is_nullable: false,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        column: col,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `updated_at` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3);"]
    );
}

#[test]
fn mysql_single_column_alter_resaving_precision_column_keeps_temporal_precision() {
    let mut col = column("updated_at");
    col.data_type = "datetime(3)".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP(3)".to_string();
    col.comment = "updated".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });
    col.original = Some(ColumnInfo {
        name: "updated_at".to_string(),
        data_type: "datetime(3)".to_string(),
        is_nullable: false,
        column_default: Some("CURRENT_TIMESTAMP(3)".to_string()),
        is_primary_key: false,
        extra: Some("on update CURRENT_TIMESTAMP(3)".to_string()),
        comment: None,
        ..Default::default()
    });

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        column: col,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `updated_at` datetime(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3) COMMENT 'updated';"]
    );
}

#[test]
fn mysql_current_timestamp_clauses_stay_bare_without_column_precision() {
    let mut col = column("updated_at");
    col.data_type = "timestamp".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });

    let result = build_create_table_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![col]));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE TABLE `users` (\n  `updated_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP\n);"]
    );

    let mut col = column("updated_at");
    col.data_type = "datetime(7)".to_string();
    col.is_nullable = false;
    col.default_value = "CURRENT_TIMESTAMP".to_string();
    col.extra = Some(ColumnExtra { on_update_current_timestamp: Some(true), ..Default::default() });

    let result = build_create_table_sql(structure_change_options(DatabaseType::Mysql, None, "users", vec![col]));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["CREATE TABLE `users` (\n  `updated_at` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP\n);"]
    );
}

#[test]
fn mysql_quotes_datetime_literal_default() {
    let mut col = column("created_at");
    col.data_type = "datetime".to_string();
    col.default_value = "2024-01-01 00:00:00".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT '2024-01-01 00:00:00'"));
}

#[test]
fn mysql_does_not_quote_current_timestamp() {
    let mut col = column("updated_at");
    col.data_type = "timestamp".to_string();
    col.default_value = "CURRENT_TIMESTAMP".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT CURRENT_TIMESTAMP"));
    assert!(!result.statements[0].contains("DEFAULT 'CURRENT_TIMESTAMP'"));
}

#[test]
fn mysql_does_not_quote_temporal_function_with_parens() {
    let mut col = column("created_at");
    col.data_type = "datetime".to_string();
    col.default_value = "NOW()".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT NOW()"));
}

#[test]
fn mysql_date_literal_default_is_quoted() {
    let mut col = column("birth_date");
    col.data_type = "date".to_string();
    col.default_value = "2000-01-01".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT '2000-01-01'"));
}

#[test]
fn mysql_time_literal_default_is_quoted() {
    let mut col = column("start_time");
    col.data_type = "time".to_string();
    col.default_value = "09:00:00".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "shifts".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT '09:00:00'"));
}

#[test]
fn non_temporal_types_are_not_quoted() {
    let mut col = column("score");
    col.data_type = "int".to_string();
    col.default_value = "0".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "games".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 0"));
    assert!(!result.statements[0].contains("DEFAULT '0'"));
}

#[test]
fn mysql_single_column_alter_quotes_datetime_literal() {
    let mut col = column("created_at");
    col.data_type = "datetime".to_string();
    col.default_value = "2024-01-01 00:00:00".to_string();
    col.original = Some(ColumnInfo {
        name: "created_at".to_string(),
        data_type: "datetime".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: Some(String::new()),
        ..Default::default()
    });

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        column: col,
    });

    assert!(result.statements.iter().any(|s| s.contains("DEFAULT '2024-01-01 00:00:00'")));
}

#[test]
fn mysql_single_generated_column_change_is_blocked_without_expression_metadata() {
    let mut generated = column("total");
    generated.data_type = "decimal(14,2)".to_string();
    generated.extra = Some(ColumnExtra::default());
    generated.original = Some(ColumnInfo {
        name: "total".to_string(),
        data_type: "decimal(12,2)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: Some("STORED GENERATED".to_string()),
        comment: None,
        ..Default::default()
    });

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "products".to_string(),
        column: generated,
    });

    assert!(result.statements.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("generation expression could not be loaded"));
}

#[test]
fn builds_mysql_foreign_key_changes() {
    let mut existing = foreign_key("fk_orders_users", "user_id", "users", "id");
    existing.on_delete = "CASCADE".to_string();
    existing.original = Some(ForeignKeyInfo {
        name: "fk_orders_users_old".to_string(),
        column: "customer_id".to_string(),
        ref_schema: None,
        ref_table: "customers".to_string(),
        ref_column: "id".to_string(),
        on_update: None,
        on_delete: Some("RESTRICT".to_string()),
    });

    let mut dropped = foreign_key("fk_orders_accounts", "account_id", "accounts", "id");
    dropped.marked_for_drop = true;
    dropped.original = Some(ForeignKeyInfo {
        name: "fk_orders_accounts".to_string(),
        column: "account_id".to_string(),
        ref_schema: None,
        ref_table: "accounts".to_string(),
        ref_column: "id".to_string(),
        on_update: None,
        on_delete: None,
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "orders".to_string(),
        columns: Vec::new(),
        indexes: Vec::new(),
        foreign_keys: vec![existing, dropped],
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `orders` DROP FOREIGN KEY `fk_orders_users_old`;",
            "ALTER TABLE `orders` ADD CONSTRAINT `fk_orders_users` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`) ON DELETE CASCADE;",
            "ALTER TABLE `orders` DROP FOREIGN KEY `fk_orders_accounts`;",
        ]
    );
}

#[test]
fn builds_mysql_composite_foreign_key() {
    let composite = foreign_key("fk_order_items_product", "tenant_id, product_id", "products", "tenant_id, id");

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "order_items".to_string(),
        columns: Vec::new(),
        indexes: Vec::new(),
        foreign_keys: vec![composite],
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `order_items` ADD CONSTRAINT `fk_order_items_product` FOREIGN KEY (`tenant_id`, `product_id`) REFERENCES `products` (`tenant_id`, `id`);",
        ]
    );
}

#[test]
fn builds_mysql_trigger_changes() {
    let mut existing = trigger("orders_bu", "BEFORE", "UPDATE", "BEGIN\n  SET NEW.updated_at = NOW();\nEND");
    existing.original = Some(TriggerInfo {
        name: "orders_bu".to_string(),
        event: "UPDATE".to_string(),
        timing: "BEFORE".to_string(),
        statement: Some("SET NEW.updated_at = CURRENT_TIMESTAMP".to_string()),
        enabled: None,
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "orders".to_string(),
        columns: Vec::new(),
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: vec![existing],
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "DROP TRIGGER `orders_bu`;",
            "CREATE TRIGGER `orders_bu` BEFORE UPDATE ON `orders` FOR EACH ROW\nBEGIN\n  SET NEW.updated_at = NOW();\nEND;",
        ]
    );
}

#[test]
fn mysql_varchar_default_is_quoted() {
    let mut col = column("name");
    col.data_type = "varchar(255)".to_string();
    col.default_value = "hello".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 'hello'"));
    assert!(!result.statements[0].contains("DEFAULT hello "));
}

#[test]
fn mysql_char_default_is_quoted() {
    let mut col = column("code");
    col.data_type = "char(10)".to_string();
    col.default_value = "abc".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "items".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 'abc'"));
}

#[test]
fn mysql_text_default_is_quoted() {
    let mut col = column("description");
    col.data_type = "text".to_string();
    col.default_value = "default value".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "products".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 'default value'"));
}

#[test]
fn mysql_enum_default_is_quoted() {
    let mut col = column("status");
    col.data_type = "enum('active','inactive')".to_string();
    col.default_value = "active".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 'active'"));
}

#[test]
fn mysql_int_default_is_not_quoted() {
    let mut col = column("score");
    col.data_type = "int".to_string();
    col.default_value = "100".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "games".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("DEFAULT 100"));
    assert!(!result.statements[0].contains("DEFAULT '100'"));
}

#[test]
fn mysql_character_column_add_with_charset_collation() {
    let mut col = column("name");
    col.data_type = "varchar(255)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` ADD COLUMN `name` varchar(255) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci`;"
        ]
    );
}

#[test]
fn single_column_alter_builder_generates_add_for_new_mysql_column() {
    let mut col = column("category");
    col.id = "ddl-preview:new:category".to_string();
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_general_ci".to_string();
    col.comment = "所属类别".to_string();

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "apis".to_string(),
        column: col,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `apis` ADD COLUMN `category` varchar(50) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_general_ci` COMMENT '所属类别';"
        ]
    );
}

#[test]
fn single_column_alter_builder_preserves_mysql_generated_expression_for_add_preview() {
    let mut col = column("total");
    col.id = "ddl-preview:existing:total".to_string();
    col.data_type = "int".to_string();
    col.original = Some(ColumnInfo {
        name: "total".to_string(),
        data_type: "int".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: Some("GENERATED ALWAYS AS (`quantity` * `price`) STORED".to_string()),
        comment: None,
        character_set: None,
        collation: None,
    });
    col.original_position = None;

    let result = build_single_column_alter_sql(SingleColumnAlterSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "orders".to_string(),
        column: col,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `orders` ADD COLUMN `total` int GENERATED ALWAYS AS (`quantity` * `price`) STORED;"]
    );
}

#[test]
fn mysql_numeric_column_omits_charset_collation_in_column_definition() {
    let mut col = column("score");
    col.data_type = "int".to_string();
    // Even if charset/collation are set on the editable column, they must NOT
    // appear in the DDL because int does not accept CHARACTER SET or COLLATE.
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "games".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements.len() == 1);
    let sql = &result.statements[0];
    assert!(!sql.contains("CHARACTER SET"));
    assert!(!sql.contains("COLLATE"));
    assert!(sql.contains("int"));
}

#[test]
fn mysql_numeric_column_ignores_charset_collation_in_change_detection() {
    // When an existing INT column has no original character_set / collation but
    // the editable draft carries stale values, the column should NOT be flagged
    // as having an attribute change.
    let mut col = column("score");
    col.data_type = "int".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();
    col.original = Some(ColumnInfo {
        name: "score".to_string(),
        data_type: "int".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "games".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    // No ALTER should be emitted — charset/collation changes on
    // non-character columns are no-ops.
    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, Vec::<String>::new());
}

#[test]
fn mysql_character_column_detects_charset_collation_change() {
    let mut col = column("name");
    col.data_type = "varchar(255)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();
    col.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `name` varchar(255) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci`;"]
    );
}

#[test]
fn mysql_character_column_preserves_charset_collation_on_other_change() {
    // Changing the default value on a character column should still
    // re-emit the charset/collation clauses so they are not lost.
    let mut col = column("name");
    col.data_type = "varchar(255)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();
    col.default_value = "guest".to_string();
    col.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(255)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_unicode_ci".to_string()),
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `name` varchar(255) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci` DEFAULT 'guest';"]
    );
}

#[test]
fn mysql_inherited_column_charset_is_omitted_from_generated_ddl() {
    // MySQL reports the effective collation of every character column, so a column
    // that simply inherits the table default looks identical to one that spells the
    // same collation out. Introspection keeps those values (the editor needs them to
    // show the column's current charset), and the redundant clauses are dropped here,
    // while generating the DDL.
    let mut col = column("note");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_0900_ai_ci".to_string();
    col.comment = "Free-form note".to_string();
    col.original = Some(ColumnInfo {
        name: "note".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `note` varchar(50) COMMENT 'Free-form note';"]
    );
}

#[test]
fn mysql_explicit_column_charset_survives_the_table_default_comparison() {
    // A column whose collation differs from the table default must keep its clauses:
    // MODIFY COLUMN replaces the whole definition, so dropping them would silently
    // convert the column to the table's default character set.
    let mut col = column("name");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_unicode_ci".to_string();
    col.comment = "Display name".to_string();
    col.original = Some(ColumnInfo {
        name: "name".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_unicode_ci".to_string()),
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` MODIFY COLUMN `name` varchar(50) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci` COMMENT 'Display name';"
        ]
    );
}

#[test]
fn mysql_inherited_column_charset_does_not_register_as_a_change() {
    // The original snapshot is normalized together with the draft, so a column left
    // untouched by the user never looks like a charset edit and produces no ALTER.
    let mut col = column("note");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_0900_ai_ci".to_string();
    col.original = Some(ColumnInfo {
        name: "note".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });
    col.original_position = Some(0);

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, Vec::<String>::new());
}

#[test]
fn mysql_collation_switched_away_from_the_table_default_is_emitted() {
    let mut col = column("note");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_bin".to_string();
    col.original = Some(ColumnInfo {
        name: "note".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });
    col.original_position = Some(0);

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `users` MODIFY COLUMN `note` varchar(50) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_bin`;"]
    );
}

#[test]
fn mysql_column_charset_switched_to_the_table_default_drops_the_clause() {
    // Omitting the clauses is equivalent to writing the table default out, so a column
    // moved onto the table default still converts — it just does so implicitly.
    let mut col = column("note");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_0900_ai_ci".to_string();
    col.original = Some(ColumnInfo {
        name: "note".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("latin1".to_string()),
        collation: Some("latin1_bin".to_string()),
    });
    col.original_position = Some(0);

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `users` MODIFY COLUMN `note` varchar(50);"]);
}

#[test]
fn mysql_column_charset_is_kept_when_the_table_default_is_unknown() {
    // Without a table default there is nothing to compare against, so the real values
    // reported by MySQL are written out rather than guessed away.
    let mut col = column("note");
    col.data_type = "varchar(50)".to_string();
    col.character_set = "utf8mb4".to_string();
    col.collation = "utf8mb4_0900_ai_ci".to_string();
    col.comment = "Free-form note".to_string();
    col.original = Some(ColumnInfo {
        name: "note".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        character_set: Some("utf8mb4".to_string()),
        collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![col],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `users` MODIFY COLUMN `note` varchar(50) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_0900_ai_ci` COMMENT 'Free-form note';"
        ]
    );
}

#[test]
fn mysql_create_table_omits_inherited_column_charset() {
    let mut inherited = column("note");
    inherited.data_type = "varchar(50)".to_string();
    inherited.character_set = "utf8mb4".to_string();
    inherited.collation = "utf8mb4_0900_ai_ci".to_string();
    let mut explicit = column("name");
    explicit.data_type = "varchar(50)".to_string();
    explicit.character_set = "utf8mb4".to_string();
    explicit.collation = "utf8mb4_unicode_ci".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "users".to_string(),
        columns: vec![inherited, explicit],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: Some("utf8mb4_0900_ai_ci".to_string()),
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "CREATE TABLE `users` (\n  `note` varchar(50),\n  `name` varchar(50) CHARACTER SET `utf8mb4` COLLATE `utf8mb4_unicode_ci`\n);"
        ]
    );
}

#[test]
fn mysql_generated_column_preserves_expression_when_modified() {
    let mut generated = column("total");
    generated.data_type = "decimal(14,2)".to_string();
    generated.is_nullable = false;
    generated.comment = "Computed total".to_string();
    generated.extra = Some(ColumnExtra::default());
    generated.original = Some(ColumnInfo {
        name: "total".to_string(),
        data_type: "decimal(12,2)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: Some("GENERATED ALWAYS AS (`price` * `quantity`) STORED".to_string()),
        comment: None,
        ..Default::default()
    });
    generated.original_position = Some(0);

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "products",
        vec![generated],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec![
            "ALTER TABLE `products` MODIFY COLUMN `total` decimal(14,2) GENERATED ALWAYS AS (`price` * `quantity`) STORED NOT NULL COMMENT 'Computed total';"
        ]
    );
}

#[test]
fn mysql_unchanged_generated_column_is_not_modified_with_other_columns() {
    let mut generated = column("total");
    generated.data_type = "decimal(12,2)".to_string();
    generated.extra = Some(ColumnExtra::default());
    generated.original = Some(ColumnInfo {
        name: "total".to_string(),
        data_type: "decimal(12,2)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: Some("STORED GENERATED".to_string()),
        comment: None,
        ..Default::default()
    });
    generated.original_position = Some(0);

    let mut status = column("status");
    status.data_type = "varchar(50)".to_string();
    status.comment = "状态1".to_string();
    status.original = Some(ColumnInfo {
        name: "status".to_string(),
        data_type: "varchar(50)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: None,
        comment: None,
        ..Default::default()
    });
    status.original_position = Some(1);

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "product_info",
        vec![generated, status],
    ));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(
        result.statements,
        vec!["ALTER TABLE `product_info` MODIFY COLUMN `status` varchar(50) COMMENT '状态1';"]
    );
}

#[test]
fn mysql_generated_column_change_is_blocked_without_expression_metadata() {
    let mut generated = column("total");
    generated.data_type = "decimal(14,2)".to_string();
    generated.extra = Some(ColumnExtra::default());
    generated.original = Some(ColumnInfo {
        name: "total".to_string(),
        data_type: "decimal(12,2)".to_string(),
        is_nullable: true,
        column_default: None,
        is_primary_key: false,
        extra: Some("STORED GENERATED".to_string()),
        comment: None,
        ..Default::default()
    });
    generated.original_position = Some(0);

    let result = build_table_structure_change_sql(structure_change_options(
        DatabaseType::Mysql,
        None,
        "products",
        vec![generated],
    ));

    assert!(result.statements.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("generation expression could not be loaded"));
}

#[test]
fn mysql_stale_concurrently_flag_is_ignored() {
    // Non-PostgreSQL engines cannot request a concurrent build at all; a stale
    // or forged `concurrently` flag must not error and must not alter the SQL.
    let mut idx = index("idx_users_email", &["email"]);
    idx.concurrently = true;

    let result = build_table_structure_change_sql(index_change_options(DatabaseType::Mysql, Some("public"), idx));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["CREATE INDEX `idx_users_email` ON `USERS` (`email`);"]);
}

#[test]
fn mysql_existing_index_with_stale_concurrently_flag_ignored() {
    // An existing-index edit on a non-PostgreSQL engine is driven by the actual
    // field changes; a stale concurrently flag alone must not force a rebuild.
    let idx = existing_index("idx_users_email", &["email"], false);
    let mut changed = idx;
    changed.concurrently = true;

    let result = build_table_structure_change_sql(index_change_options(DatabaseType::Mysql, Some("public"), changed));

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements.is_empty(), "no rebuild for a flag-only change, got: {:?}", result.statements);
}

#[test]
fn mysql_create_table_nullable_timestamp_without_default_gets_explicit_null() {
    // A second (or later) MySQL TIMESTAMP column that is nullable but has no
    // DEFAULT must carry an explicit NULL keyword — otherwise MySQL either
    // silently rewrites it to NOT NULL or, with the still-common
    // explicit_defaults_for_timestamp=OFF server default, rejects it outright
    // with ERROR 1067 (42000): Invalid default value. See issue #7416.
    let mut created_at = column("created_at");
    created_at.data_type = "timestamp".to_string();
    created_at.is_nullable = false;

    let mut updated_at = column("updated_at");
    updated_at.data_type = "timestamp".to_string();
    updated_at.is_nullable = true;

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "u7_game_order_step".to_string(),
        columns: vec![created_at, updated_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("`updated_at` timestamp NULL"));
    assert!(!result.statements[0].contains("`updated_at` timestamp NULL DEFAULT"));
}

#[test]
fn mysql_create_table_nullable_timestamp_with_default_still_gets_explicit_null() {
    // Even with an explicit DEFAULT, MySQL still needs the NULL keyword to
    // keep the column nullable — omitting it silently produces NOT NULL.
    let mut updated_at = column("updated_at");
    updated_at.data_type = "timestamp".to_string();
    updated_at.is_nullable = true;
    updated_at.default_value = "CURRENT_TIMESTAMP".to_string();

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        columns: vec![updated_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(result.statements[0].contains("`updated_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP"));
}

#[test]
fn mysql_create_table_nullable_datetime_does_not_gain_null_keyword() {
    // DATETIME is not subject to MySQL's TIMESTAMP-specific implicit-default
    // quirk; the fix must not touch it.
    let mut updated_at = column("updated_at");
    updated_at.data_type = "datetime".to_string();
    updated_at.is_nullable = true;

    let result = build_create_table_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "events".to_string(),
        columns: vec![updated_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert!(!result.statements[0].contains("NULL"));
}

#[test]
fn mysql_add_column_nullable_timestamp_without_default_gets_explicit_null() {
    // The ADD COLUMN / MODIFY COLUMN / CHANGE COLUMN path shares
    // column_definition() with CREATE TABLE and must carry the same fix.
    let mut updated_at = column("updated_at");
    updated_at.data_type = "timestamp".to_string();
    updated_at.is_nullable = true;

    let result = build_table_structure_change_sql(TableStructureSqlOptions {
        database_type: Some(DatabaseType::Mysql),
        driver_profile: None,
        schema: None,
        table_name: "u7_game_order_step".to_string(),
        columns: vec![updated_at],
        indexes: Vec::new(),
        foreign_keys: Vec::new(),
        triggers: Vec::new(),
        table_comment: None,
        original_table_comment: None,
        mysql_engine: None,
        transwarp_create: None,
        partitioned: false,
        is_gaussdb_m_mode: false,
        table_collation: None,
    });

    assert_eq!(result.warnings, Vec::<String>::new());
    assert_eq!(result.statements, vec!["ALTER TABLE `u7_game_order_step` ADD COLUMN `updated_at` timestamp NULL;"]);
}
