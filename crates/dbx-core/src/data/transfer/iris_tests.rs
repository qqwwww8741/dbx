use super::*;
use serde_json::json;

const IRIS_TRANSFER_AGENT: &str = r#"
import json, pathlib, re, sys
trace = pathlib.Path(sys.argv[1])
row_count = int(sys.argv[2])
failure = sys.argv[3]
has_key = sys.argv[4] == 'true'

def query(rows=None, columns=None):
    return {'columns': columns or [], 'column_types': [], 'column_sortables': [],
            'rows': rows or [], 'affected_rows': 1, 'execution_time_ms': 0,
            'truncated': False, 'session_id': None, 'has_more': False}

print(json.dumps({'ready': True}), flush=True)
for line in sys.stdin:
    req = json.loads(line)
    method = req['method']
    params = req.get('params', {})
    sql = params.get('sql', '')
    with trace.open('a', encoding='utf-8') as output:
        output.write(json.dumps({'method': method, 'sql': sql, 'params': params}) + '\n')
    try:
        if method == 'handshake':
            result = {'protocolVersion': 2, 'agentProtocolVersion': 2,
                      'capabilities': ['multi_session', 'query', 'metadata']}
        elif method == 'list_tables':
            result = [{'name': 'items', 'table_type': 'TABLE'}]
        elif method == 'get_columns':
            result = [{'name': name, 'data_type': kind, 'is_nullable': False,
                       'is_primary_key': has_key and name == 'id'}
                      for name, kind in [('id', 'INTEGER'), ('name', 'VARCHAR')]]
        elif method == 'execute_query':
            if sql.startswith('SELECT COUNT('):
                if failure == 'count':
                    raise ValueError('injected count failure')
                result = query([[row_count]], ['count'])
            elif sql.startswith('SELECT'):
                top = re.search(r'SELECT TOP (\d+)', sql)
                if not top or ' LIMIT ' in sql or ' OFFSET ' in sql:
                    raise ValueError('expected TOP/VID pagination: ' + sql)
                expected_order = 'ORDER BY "id"' if has_key else 'ORDER BY %ID'
                if expected_order not in sql:
                    raise ValueError('expected stable table order: ' + sql)
                offset = re.search(r'%VID > (\d+)', sql)
                offset = int(offset[1]) if offset else 0
                if failure == 'source' and offset > 0:
                    raise ValueError('injected source failure')
                rows = [[n, 'row-' + str(n)] for n in range(1, row_count + 1)]
                result = query(rows[offset:int(top[1])], ['id', 'name'])
            elif sql.startswith('INSERT INTO'):
                if '),\n(' in sql or 'MERGE' in sql or 'ON CONFLICT' in sql:
                    raise ValueError('expected single-row INSERT: ' + sql)
                if failure == 'target' and 'VALUES\n(3,' in sql:
                    raise ValueError('injected target failure')
                result = query()
            elif sql.startswith('TRUNCATE TABLE'):
                result = query()
            else:
                raise ValueError('unexpected transfer SQL: ' + sql)
        else:
            result = {'ok': True}
        response = {'jsonrpc': '2.0', 'id': req['id'], 'result': result}
    except Exception as error:
        response = {'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -1, 'message': str(error)}}
    print(json.dumps(response), flush=True)
"#;
