import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { discoverRecipes, validateRecipe, validateHostPortAllocation, defaultHostPortMappings, allPortMappingsDefaultToLoopback, dbxConnectionDeepLink, resolveRecipe, parseDatabaseSelection, expandSmokeCommand, assertResetConfirmed } from './database-env.mjs';

const recipes = discoverRecipes();

test('only MySQL recipes are shipped and each is valid', () => {
  assert.ok(recipes.length > 0);
  for (const recipe of recipes) {
    assert.equal(recipe.database, 'mysql');
    assert.deepEqual(validateRecipe(recipe), [], recipe.displayVersion);
  }
  assert.deepEqual(validateHostPortAllocation(recipes), []);
});

test('every MySQL recipe binds its declared port to loopback', () => {
  for (const recipe of recipes) {
    const compose = readFileSync(join(recipe.directory, 'compose.yaml'), 'utf8');
    assert.ok(allPortMappingsDefaultToLoopback(compose));
    assert.equal(defaultHostPortMappings(compose)[0].hostPort, recipe.connection.port);
  }
});

test('MySQL deep links preserve reserved characters and connection overrides', () => {
  for (const recipe of recipes) {
    const params = new URL(dbxConnectionDeepLink(recipe, { DB_PORT: '13306', DB_PASSWORD: 'p@ss & word' })).searchParams;
    assert.equal(params.get('type'), 'mysql');
    assert.equal(params.get('port'), '13306');
    assert.equal(params.get('password'), 'p@ss & word');
    assert.equal(params.get('user'), 'root');
    assert.equal(params.get('database'), 'dbx');
  }
});

test('version selection requires an unambiguous MySQL version', () => {
  assert.throws(() => resolveRecipe(recipes, ''), /DB is required/);
  if (recipes.length > 1) assert.throws(() => resolveRecipe(recipes, 'mysql'), /multiple versions/);
  for (const recipe of recipes) assert.equal(resolveRecipe(recipes, 'mysql', recipe.displayVersion), recipe);
  assert.deepEqual(parseDatabaseSelection('mysql@8.4'), { database: 'mysql', version: '8.4' });
  assert.throws(() => parseDatabaseSelection('mysql@8.4', '5.7'), /conflicts/);
});

test('smoke command arguments preserve password overrides', () => {
  const recipe = { connection: { password: 'default', port: 3306 } };
  assert.deepEqual(expandSmokeCommand(['mysql', '--password=${DB_PASSWORD}', '--port=${DB_PORT}'], recipe, { DB_PASSWORD: 'p@ss & word', DB_PORT: '13306' }), ['mysql', '--password=p@ss & word', '--port=13306']);
});

test('resetting test data requires explicit confirmation', () => {
  assert.throws(() => assertResetConfirmed(''), /CONFIRM=1/);
  assert.doesNotThrow(() => assertResetConfirmed('1'));
});
