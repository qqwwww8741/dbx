import { type DatabaseType, type QueryResult } from "@/types/database";

export interface ElasticsearchJsonResponse {
  status: number;
  body: string;
}

/**
 * Detect the raw HTTP result emitted for a search-engine REST request.
 * Elasticsearch asks unformatted CAT requests for JSON; Solr REST passthrough
 * emits the same `status`/`response` shape, so both share this response panel.
 */
export function elasticsearchJsonResponseForResult(_databaseType: DatabaseType | undefined, _sourceStatement: string | undefined, _result: QueryResult | undefined): ElasticsearchJsonResponse | undefined {
  {
    return undefined;
  }
}
