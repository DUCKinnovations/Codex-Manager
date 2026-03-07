import test from "node:test";
import assert from "node:assert/strict";

import {
  buildEnvOverrideDescription,
  buildEnvOverrideOptionLabel,
  filterEnvOverrideCatalog,
  formatEnvOverrideDisplayValue,
  normalizeEnvOverrideCatalog,
  normalizeEnvOverrides,
  normalizeStringList,
} from "../env-overrides.js";

test("normalize helpers keep deterministic env maps and arrays", () => {
  assert.deepEqual(normalizeStringList([" b ", "a", "", "a"]), ["a", "b"]);
  assert.deepEqual(
    normalizeEnvOverrides({
      foo: "bar",
      CODEXMANAGER_UPSTREAM_BASE_URL: " https://chatgpt.com ",
      CODEXMANAGER_UPSTREAM_COOKIE: "",
    }),
    {
      CODEXMANAGER_UPSTREAM_BASE_URL: "https://chatgpt.com",
      CODEXMANAGER_UPSTREAM_COOKIE: "",
    },
  );
});

test("normalizeEnvOverrideCatalog keeps label and defaultValue", () => {
  assert.deepEqual(
    normalizeEnvOverrideCatalog([
      {
        key: "CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS",
        label: "上游总超时",
        scope: "service",
        applyMode: "runtime",
        defaultValue: 120000,
      },
      {
        key: "codexmanager_web_root",
        scope: "web",
        applyMode: "restart",
        defaultValue: "",
      },
    ]),
    [
      {
        key: "CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS",
        label: "上游总超时",
        scope: "service",
        applyMode: "runtime",
        defaultValue: "120000",
      },
      {
        key: "CODEXMANAGER_WEB_ROOT",
        label: "CODEXMANAGER_WEB_ROOT",
        scope: "web",
        applyMode: "restart",
        defaultValue: "",
      },
    ],
  );
});

test("filterEnvOverrideCatalog supports label and key search", () => {
  const catalog = normalizeEnvOverrideCatalog([
    {
      key: "CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS",
      label: "上游总超时",
      scope: "service",
      applyMode: "runtime",
      defaultValue: "120000",
    },
    {
      key: "CODEXMANAGER_PROMPT_CACHE_TTL_SECS",
      label: "提示缓存 TTL",
      scope: "service",
      applyMode: "runtime",
      defaultValue: "3600",
    },
  ]);

  assert.deepEqual(
    filterEnvOverrideCatalog(catalog, "缓存").map((item) => item.key),
    ["CODEXMANAGER_PROMPT_CACHE_TTL_SECS"],
  );
  assert.deepEqual(
    filterEnvOverrideCatalog(catalog, "timeout").map((item) => item.key),
    ["CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS"],
  );
});

test("display helpers format empty values predictably", () => {
  const item = {
    key: "CODEXMANAGER_UPSTREAM_TOTAL_TIMEOUT_MS",
    label: "上游总超时",
  };
  assert.equal(buildEnvOverrideOptionLabel(item), "上游总超时");
  assert.match(buildEnvOverrideDescription(item), /最长时间|超时/);
  assert.match(
    buildEnvOverrideDescription({
      key: "CODEXMANAGER_PROMPT_CACHE_CAPACITY",
      label: "Prompt 缓存容量",
    }),
    /容量上限/,
  );
  assert.equal(formatEnvOverrideDisplayValue(""), "空");
  assert.equal(formatEnvOverrideDisplayValue("  600 "), "600");
});
