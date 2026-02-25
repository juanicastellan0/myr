use super::{RelationshipDirection, SchemaCatalog, TableRelationship};

pub(super) fn collect_table_relationships(
    schema: &SchemaCatalog,
    database_name: &str,
    table_name: &str,
) -> Vec<TableRelationship> {
    let mut relationships = Vec::new();

    let Some(database) = schema.database(database_name) else {
        return relationships;
    };

    if let Some(table) = database
        .tables
        .iter()
        .find(|table| table.name == table_name)
    {
        for foreign_key in &table.foreign_keys {
            relationships.push(TableRelationship {
                direction: RelationshipDirection::Outbound,
                constraint_name: foreign_key.constraint_name.clone(),
                source_column: foreign_key.column_name.clone(),
                related_database: foreign_key.referenced_database.clone(),
                related_table: foreign_key.referenced_table.clone(),
                related_column: foreign_key.referenced_column.clone(),
            });
        }
    }

    for candidate_table in &database.tables {
        for foreign_key in &candidate_table.foreign_keys {
            if foreign_key.referenced_table == table_name
                && foreign_key.referenced_database == database_name
            {
                relationships.push(TableRelationship {
                    direction: RelationshipDirection::Inbound,
                    constraint_name: foreign_key.constraint_name.clone(),
                    source_column: foreign_key.referenced_column.clone(),
                    related_database: database_name.to_string(),
                    related_table: candidate_table.name.clone(),
                    related_column: foreign_key.column_name.clone(),
                });
            }
        }
    }

    relationships.sort_unstable_by(|left, right| {
        left.related_database
            .cmp(&right.related_database)
            .then_with(|| left.related_table.cmp(&right.related_table))
            .then_with(|| left.related_column.cmp(&right.related_column))
            .then_with(|| left.constraint_name.cmp(&right.constraint_name))
            .then_with(|| left.direction.cmp(&right.direction))
    });

    relationships
}
