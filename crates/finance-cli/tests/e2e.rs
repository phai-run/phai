use assert_cmd::prelude::*;
use chrono::{Duration, Utc};
use predicates::prelude::*;
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    Command::cargo_bin("finance-cli").expect("finance-cli binary")
}

fn envs<'a>(cmd: &'a mut Command, config_dir: &Path, data_dir: &Path) -> &'a mut Command {
    cmd.env("FINANCE_OS_CONFIG_DIR", config_dir)
        .env("FINANCE_OS_DATA_DIR", data_dir)
        // Tests must never trigger the self-updater: it overwrites the
        // very `target/debug/finance-cli` binary cargo just built, causing
        // subsequent assertions to run against an old release artifact
        // and producing false negatives.
        .env("FINANCE_OS_NO_AUTO_UPDATE", "1")
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn setup_local_auth_migrate(config_dir: &Path, data_dir: &Path) {
    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        config_dir,
        data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        config_dir,
        data_dir,
    )
    .assert()
    .success();
}

fn seed_fixture_sync(temp: &TempDir, config_dir: &Path, data_dir: &Path) {
    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31"),
        config_dir,
        data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- transactions: 4"));
}

#[test]
fn transaction_anatomy_fields_and_pending_commands_work() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(&db_path).expect("open db");
    let (raw, description): (String, Option<String>) = conn
        .query_row(
            "SELECT raw_description, description
             FROM transactions
             WHERE transaction_id = 'pluggy-fixture-001'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("anatomy row");
    assert_eq!(raw, "Supermercado Angeloni");
    assert!(description.is_none());

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-anatomy")
            .arg("--transaction-id")
            .arg("pluggy-fixture-001")
            .arg("--description")
            .arg("Compra mensal de mercado")
            .arg("--merchant-name")
            .arg("Mercado Exemplo")
            .arg("--purpose")
            .arg("Reposição da cozinha"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let (description, merchant, purpose): (String, String, String) = conn
        .query_row(
            "SELECT description, merchant_name, purpose
             FROM transactions
             WHERE transaction_id = 'pluggy-fixture-001'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("updated anatomy row");
    assert_eq!(description, "Compra mensal de mercado");
    assert_eq!(merchant, "Mercado Exemplo");
    assert_eq!(purpose, "Reposição da cozinha");

    let pending = envs(
        cargo_bin()
            .arg("tx")
            .arg("pending-human")
            .arg("--kind")
            .arg("description")
            .arg("--json")
            .arg("--limit")
            .arg("20"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let rows: Value = serde_json::from_slice(&pending).expect("pending json");
    assert!(rows
        .as_array()
        .expect("array")
        .iter()
        .all(|row| row["transaction_id"] != "pluggy-fixture-001"));

    let review_summary = envs(
        cargo_bin()
            .arg("tx")
            .arg("review-human")
            .arg("--summary")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let summary: Value = serde_json::from_slice(&review_summary).expect("review summary json");
    assert_eq!(summary["missingDescriptionCount"], 3);
    assert_eq!(summary["missingMerchantCount"], 3);
    assert!(summary["suggestedNextCommand"]
        .as_str()
        .expect("next command")
        .contains("tx review-human --kind all"));

    let review_queue = envs(
        cargo_bin()
            .arg("tx")
            .arg("review-human")
            .arg("--kind")
            .arg("all")
            .arg("--json")
            .arg("--limit")
            .arg("20"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let review_rows: Value = serde_json::from_slice(&review_queue).expect("review queue json");
    assert!(review_rows
        .as_array()
        .expect("array")
        .iter()
        .any(|row| row["transaction_id"] != "pluggy-fixture-001"));

    let review_result = envs(
        cargo_bin()
            .arg("tx")
            .arg("review-human")
            .arg("--transaction-id")
            .arg("pluggy-fixture-003")
            .arg("--description")
            .arg("Refeição revisada")
            .arg("--merchant-name")
            .arg("Restaurante Exemplo")
            .arg("--category")
            .arg("alimentacao:restaurantes")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let applied_rows: Value = serde_json::from_slice(&review_result).expect("review result json");
    let applied = applied_rows
        .as_array()
        .expect("review result array")
        .first()
        .expect("one applied result");
    assert_eq!(applied["transactionId"], "pluggy-fixture-003");
    assert_eq!(applied["updatedDescription"], true);
    assert_eq!(applied["updatedMerchantName"], true);
    assert_eq!(applied["updatedCategory"], true);

    let (description, merchant, category): (String, String, String) = conn
        .query_row(
            "SELECT description, merchant_name, category_id
             FROM transactions
             WHERE transaction_id = 'pluggy-fixture-003'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("reviewed anatomy row");
    assert_eq!(description, "Refeição revisada");
    assert_eq!(merchant, "Restaurante Exemplo");
    assert_eq!(category, "alimentacao:restaurantes");
}

#[test]
fn milestone_zero_local_sync_and_report() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("local-dev"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // The new pulse format is a proactive headline-driven summary; it
    // does not list individual transactions in the body. The `--raw`
    // variant below is what scripts/skills should consume for the
    // backwards-compatible per-transaction view.
    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("120"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Pulso"))
    .stdout(predicate::str::contains("Mês até dia"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("120")
            .arg("--raw"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Supermercado Angeloni"))
    .stdout(predicate::str::contains("Pagamento recebido"));
}

#[test]
fn split_commands_are_bigquery_only_on_local_backend() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let unsupported = "transaction split/detailing is supported only on the BigQuery backend";

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    for args in [
        vec![
            "tx",
            "split",
            "preview",
            "--transaction-id",
            "tx-1",
            "--payload",
            "missing.json",
        ],
        vec![
            "tx",
            "split",
            "apply",
            "--transaction-id",
            "tx-1",
            "--payload",
            "missing.json",
        ],
        vec!["tx", "split", "show", "--transaction-id", "tx-1"],
        vec![
            "tx",
            "split",
            "clear",
            "--transaction-id",
            "tx-1",
            "--reason",
            "test",
        ],
        vec!["report", "split-candidates", "--json"],
        vec!["report", "item-prices", "--query", "leite", "--json"],
    ] {
        envs(cargo_bin().args(args), &config_dir, &data_dir)
            .assert()
            .failure()
            .stderr(predicate::str::contains(unsupported));
    }
}

#[test]
fn import_legacy_is_idempotent() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    let finance_root = temp.path().join("legacy");
    fs::create_dir_all(finance_root.join("data/2026")).expect("create legacy data dir");

    write_file(
        &finance_root.join("data/contas.csv"),
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\n",
    );
    write_file(
        &finance_root.join("data/2026/transacoes.csv"),
        "pluggy_id,data,mes_ref,mes_movimento_ref,fatura_mes_ref,data_fechamento_fatura,data_vencimento,data_pagamento,status_pagamento,competencia_tipo,conta_id,conta_owner,conta_tipo,conta_banco,conta_label,valor,descricao,descricao_raw,categoria,subcategoria,descricao_canonica,contexto_finalidade,recorrencia_tipo,recorrencia_chave,recorrencia_frequencia,recorrencia_dia,recorrencia_origem,classificacao_fonte,classificacao_regra,tipo,data_hora_iso,pluggy_account_id,pluggy_amount_raw,pluggy_currency_code,pluggy_amount_in_account_currency,pluggy_balance,pluggy_category,pluggy_category_id,pluggy_provider_code,pluggy_provider_id,pluggy_status,pluggy_operation_type,pluggy_order,pluggy_created_at,pluggy_updated_at,payment_data_json,payment_method,payment_reason,payment_reference_number,payment_receiver_reference_id,payment_boleto_digitable_line,payment_boleto_barcode,payment_boleto_base_amount,payment_boleto_interest_amount,payment_boleto_penalty_amount,payment_boleto_discount_amount,payer_name,payer_branch_number,payer_account_number,payer_routing_number,payer_routing_number_ispb,payer_document_type,payer_document_value,receiver_name,receiver_branch_number,receiver_account_number,receiver_routing_number,receiver_routing_number_ispb,receiver_document_type,receiver_document_value,credit_card_metadata_json,credit_card_card_number,credit_card_payee_mcc,credit_card_installment_number,credit_card_total_installments,credit_card_total_amount,credit_card_purchase_date,credit_card_bill_id,merchant_json,merchant_name,merchant_business_name,merchant_cnpj,merchant_cnae,merchant_category,acquirer_data_json,raw_transaction_json\npluggy-1,2026-03-01,2026-03,2026-03,,,,,pago,conta_corrente,primary_checking,primary,checking,fintech,Primary Checking,-42.50,Mercado,Mercado,Alimentacao,Mercado,,, ,,,, ,rule,mercado_rule,DEBIT,2026-03-01T12:00:00.000Z,fixture-checking,,,,,,,,,POSTED,,,,2026-03-01T12:00:00.000Z,2026-03-01T12:00:00.000Z,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,,\n",
    );
    write_file(
        &finance_root.join("contexto_transacoes.csv"),
        "match_type,match_value,valor_match,categoria,subcategoria,rotulo,finalidade,tipo_contexto,notas,ativo,revisado_em\ndescricao_contains,mercado,,Alimentacao,Mercado,Mercado do mês,Compras de mercado,contexto,,1,2026-03-01\n",
    );
    write_file(
        &finance_root.join("data/forecast_templates.csv"),
        "id,tipo,descricao,categoria,subcategoria,conta_id,valor,frequencia,dia_vencimento,inicio_mes,fim_mes,parcelas_total,match_contains,status,impacta_total,origem,notas\naluguel,recorrente,Aluguel,Moradia,Aluguel,primary_checking,1000.00,mensal,5,2026-03,2026-12,,aluguel,ativo,true,teste,\n",
    );
    write_file(
        &finance_root.join("rules.yaml"),
        "version: 1\ncategories:\n  - id: mercado_rule\n    match:\n      contains_any: [\"mercado\"]\n    set:\n      category: Alimentacao\n      subcategory: Mercado\n",
    );

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    for _ in 0..2 {
        envs(
            cargo_bin()
                .arg("admin")
                .arg("import-legacy")
                .arg("--finance-root")
                .arg(&finance_root),
            &config_dir,
            &data_dir,
        )
        .assert()
        .success()
        .stdout(predicate::str::contains("- transactions: 1"));
    }

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let tx_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM transactions", [], |row| row.get(0))
        .expect("count transactions");
    let rule_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM rules", [], |row| row.get(0))
        .expect("count rules");
    let forecast_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM forecast", [], |row| row.get(0))
        .expect("count forecast");

    assert_eq!(tx_count, 1);
    assert_eq!(rule_count, 2);
    assert_eq!(forecast_count, 1);
}

#[test]
fn mutating_commands_feed_reporting_views() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("account")
            .arg("upsert")
            .arg("--account-id")
            .arg("reserva_emergencia")
            .arg("--owner")
            .arg("primary")
            .arg("--account-type")
            .arg("savings")
            .arg("--bank")
            .arg("itau")
            .arg("--label")
            .arg("Reserva Emergencia"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Conta salva: reserva_emergencia"));

    envs(
        cargo_bin()
            .arg("rule")
            .arg("upsert")
            .arg("--rule-id")
            .arg("farmacia_rule")
            .arg("--body")
            .arg("if description contains farmacia then category saude:farmacia"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Rule salva: farmacia_rule"));

    envs(
        cargo_bin()
            .arg("forecast")
            .arg("upsert")
            .arg("--forecast-id")
            .arg("mercado_planejado")
            .arg("--date")
            .arg("2026-03-30")
            .arg("--description")
            .arg("Mercado planejado")
            .arg("--amount")
            .arg("200.00")
            .arg("--category")
            .arg("Alimentacao")
            .arg("--subcategory")
            .arg("Mercado")
            .arg("--account-id")
            .arg("primary_checking"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Forecast salvo: mercado_planejado",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-card-001")
            .arg("--account-id")
            .arg("shared_credit")
            .arg("--date")
            .arg("2026-03-27")
            .arg("--description")
            .arg("Farmacia Cartao")
            .arg("--amount=-87.45")
            .arg("--payment-status")
            .arg("pending"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Transação manual salva: manual-card-001",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--account-id")
            .arg("primary_checking")
            .arg("--date")
            .arg("2026-03-27")
            .arg("--description")
            .arg("Exame Laboratorio")
            .arg("--amount=-120.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "Transação manual salva: manual-checking-001",
    ));

    envs(
        cargo_bin()
            .arg("tx")
            .arg("categorize")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--category")
            .arg("Saude")
            .arg("--subcategory")
            .arg("Exames")
            .arg("--context")
            .arg("Exame de rotina"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context")
            .arg("--transaction-id")
            .arg("manual-checking-001")
            .arg("--context")
            .arg("Exame de rotina anual"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("report")
            .arg("monthly-spend")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    // human format: family groups use display labels (capitalized, with accents)
    .stdout(predicate::str::contains("Alimentação"))
    .stdout(predicate::str::contains("Saúde"))
    .stdout(predicate::str::contains("financeiro-pagamento-recebido").not());

    envs(
        cargo_bin()
            .arg("report")
            .arg("cashflow")
            .arg("--months")
            .arg("1"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    // human format: month shown as "março/2026" not "2026-03"
    .stdout(predicate::str::contains("março/2026"))
    .stdout(predicate::str::contains("líquido"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("forecast-vs-actual")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Mercado planejado"))
    .stdout(predicate::str::contains("realizado"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("card-summary")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("shared_credit"))
    .stdout(predicate::str::contains("Cartões"));

    envs(
        cargo_bin()
            .arg("report")
            .arg("uncategorized")
            .arg("--limit")
            .arg("10"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Farmacia Cartao"))
    .stdout(predicate::str::contains("Exame Laboratorio").not());

    let health = envs(
        cargo_bin()
            .arg("report")
            .arg("data-health")
            .arg("--days")
            .arg("120")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("data-health output");
    assert!(health.status.success());
    let health_json: Value = serde_json::from_slice(&health.stdout).expect("data-health json");
    assert_eq!(health_json["uncategorizedCount"], 1);
    assert_eq!(health_json["windowPluggyRows"], 4);
    assert_eq!(health_json["windowOtherRows"], 2);
    assert!(health_json["flatCategoryRows"].as_u64().unwrap() > 0);
    assert_eq!(health_json["overlapCandidatesCount"], 0);

    let scenario = envs(
        cargo_bin()
            .arg("report")
            .arg("scenario")
            .arg("--month")
            .arg("2026-04")
            .arg("--history-months")
            .arg("1")
            .arg("--extra-expense")
            .arg("80")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("scenario output");
    assert!(scenario.status.success());
    let scenario_json: Value = serde_json::from_slice(&scenario.stdout).expect("scenario json");
    assert_eq!(scenario_json["targetMonth"], "2026-04");
    assert_eq!(scenario_json["baselineMonths"][0], "2026-03");
    assert_eq!(scenario_json["extraExpense"], "80");
    assert_ne!(scenario_json["carryoverOpenCardAmount"], "0");

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let saved_context: String = conn
        .query_row(
            "SELECT context FROM transactions WHERE transaction_id = 'manual-checking-001'",
            [],
            |row| row.get(0),
        )
        .expect("saved context");
    let saved_category: String = conn
        .query_row(
            "SELECT category_id FROM transactions WHERE transaction_id = 'manual-checking-001'",
            [],
            |row| row.get(0),
        )
        .expect("saved category");
    let account_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM accounts WHERE account_id = 'reserva_emergencia'",
            [],
            |row| row.get(0),
        )
        .expect("count account");
    let rule_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rules WHERE rule_id = 'farmacia_rule'",
            [],
            |row| row.get(0),
        )
        .expect("count rule");
    let forecast_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM forecast WHERE forecast_id = 'mercado_planejado'",
            [],
            |row| row.get(0),
        )
        .expect("count forecast");

    assert_eq!(saved_context, "Exame de rotina anual");
    assert_eq!(saved_category, "saude:exames");
    assert_eq!(account_count, 1);
    assert_eq!(rule_count, 1);
    assert_eq!(forecast_count, 1);

    // Credit-card debit from Pluggy (positive +150) must be stored as negative
    let cc_debit_amount: String = conn
        .query_row(
            "SELECT amount FROM transactions WHERE transaction_id = 'pluggy-fixture-003'",
            [],
            |row| row.get(0),
        )
        .expect("credit card debit amount");
    let cc_debit_val: f64 = cc_debit_amount.parse().expect("parse cc debit");
    assert!(
        cc_debit_val < 0.0,
        "credit-card debit must be negated, got {cc_debit_amount}"
    );
    assert!(
        (cc_debit_val - (-150.0)).abs() < 0.01,
        "credit-card debit must be -150, got {cc_debit_amount}"
    );

    // Credit-card credit (refund +42.50) must stay positive
    let cc_credit_amount: String = conn
        .query_row(
            "SELECT amount FROM transactions WHERE transaction_id = 'pluggy-fixture-004'",
            [],
            |row| row.get(0),
        )
        .expect("credit card credit amount");
    let cc_credit_val: f64 = cc_credit_amount.parse().expect("parse cc credit");
    assert!(
        cc_credit_val > 0.0,
        "credit-card credit must stay positive, got {cc_credit_amount}"
    );
}

#[test]
fn report_card_closed_insights_includes_categories_recurring_subscriptions_and_installments() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let manual_rows = [
        (
            "cc-rec-2026-01",
            "2026-01-18",
            "Academia Fit",
            "-180.00",
            Some(("Saude", "Academia")),
            "posted",
        ),
        (
            "cc-rec-2026-02",
            "2026-02-18",
            "Academia Fit",
            "-180.00",
            Some(("Saude", "Academia")),
            "posted",
        ),
        (
            "cc-rec-2026-03",
            "2026-03-18",
            "Academia Fit",
            "-180.00",
            Some(("Saude", "Academia")),
            "posted",
        ),
        (
            "cc-sub-2026-03",
            "2026-03-21",
            "Assinatura Cloud Backup",
            "-49.90",
            Some(("Assinaturas", "Cloud")),
            "posted",
        ),
        (
            "cc-inst-2026-03",
            "2026-03-22",
            "Notebook Pro 03/10",
            "-450.00",
            Some(("Tecnologia", "Eletronicos")),
            "posted",
        ),
        (
            "cc-inst-2026-03-parc",
            "2026-03-23",
            "Yelumseg Parc8",
            "-197.71",
            Some(("Transporte", "Seguro Auto")),
            "posted",
        ),
        (
            "cc-inst-open-2026-03",
            "2026-03-24",
            "Amazon Marketplace Parcela 4/6",
            "-145.60",
            Some(("Shopping", "Marketplace")),
            "pending",
        ),
    ];

    for (tx_id, date, description, amount, category, payment_status) in manual_rows {
        let mut cmd = cargo_bin();
        cmd.arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg(tx_id)
            .arg("--account-id")
            .arg("shared_credit")
            .arg("--date")
            .arg(date)
            .arg("--description")
            .arg(description)
            .arg(format!("--amount={amount}"))
            .arg("--payment-status")
            .arg(payment_status);
        if let Some((cat, sub)) = category {
            cmd.arg("--category").arg(cat).arg("--subcategory").arg(sub);
        }
        envs(&mut cmd, &config_dir, &data_dir).assert().success();
    }

    envs(
        cargo_bin()
            .arg("report")
            .arg("card-closed-insights")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Fatura fechada"))
    .stdout(predicate::str::contains("Recorrentes"))
    .stdout(predicate::str::contains("Assinaturas"))
    .stdout(predicate::str::contains("Parcelamentos"))
    .stdout(predicate::str::contains("academia fit"))
    .stdout(predicate::str::contains("03/10"))
    .stdout(predicate::str::contains("parcela-8"))
    .stdout(predicate::str::contains("4/6"));

    let json_output = envs(
        cargo_bin()
            .arg("report")
            .arg("card-closed-insights")
            .arg("--month")
            .arg("2026-03")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("card-closed-insights json output");
    assert!(json_output.status.success());
    let payload: Value = serde_json::from_slice(&json_output.stdout).expect("valid json payload");
    assert_eq!(payload["monthRef"], "2026-03");
    assert!(!payload["accounts"].as_array().unwrap().is_empty());
    assert!(!payload["categories"].as_array().unwrap().is_empty());
    assert!(!payload["recurring"].as_array().unwrap().is_empty());
    assert!(!payload["subscriptions"].as_array().unwrap().is_empty());
    assert!(!payload["closedInstallments"].as_array().unwrap().is_empty());
    assert!(!payload["openInstallments"].as_array().unwrap().is_empty());
    assert!(payload["closedInstallments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["marker"] == "parcela-8"));
    assert!(payload["openInstallments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["marker"] == "4/6"));
}

#[test]
fn report_views_exclude_legacy_manual_statement_when_pluggy_match_exists() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual_statement_test_dup")
            .arg("--account-id")
            .arg("shared_credit")
            .arg("--date")
            .arg("2026-03-26")
            .arg("--description")
            .arg("Posto de Gasolina")
            .arg("--amount=-150.00")
            .arg("--payment-status")
            .arg("posted"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    conn.execute(
        "UPDATE transactions SET source = 'legacy' WHERE transaction_id = 'manual_statement_test_dup'",
        [],
    )
    .expect("set manual source as legacy");

    let json_output = envs(
        cargo_bin()
            .arg("report")
            .arg("monthly-spend")
            .arg("--month")
            .arg("2026-03")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("monthly-spend json output");
    assert!(json_output.status.success());
    let payload: Value = serde_json::from_slice(&json_output.stdout).expect("valid json payload");
    let sum_expenses = payload
        .as_array()
        .expect("array payload")
        .iter()
        .map(|item| {
            item["expenses"]
                .as_str()
                .expect("expense string")
                .parse::<f64>()
                .expect("expense decimal")
        })
        .sum::<f64>();
    assert!(
        (sum_expenses - 302.30).abs() < 0.01,
        "monthly spend must ignore deduped manual statement row; got {sum_expenses}"
    );
}

#[test]
fn cashflow_and_card_summary_exclude_legacy_manual_shadow() {
    // Regression: v_cashflow and v_card_summary must honour the same
    // legacy/manual ↔ pluggy dedup filter applied by v_transactions_reportable.
    // A previous refactor pointed these aggregates directly at `transactions`,
    // which caused shadowed manual statement rows to be counted twice.
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual_statement_cashflow_dup")
            .arg("--account-id")
            .arg("shared_credit")
            .arg("--date")
            .arg("2026-03-26")
            .arg("--description")
            .arg("Posto de Gasolina")
            .arg("--amount=-150.00")
            .arg("--payment-status")
            .arg("posted"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(&db_path).expect("open db");
    conn.execute(
        "UPDATE transactions SET source = 'legacy' WHERE transaction_id = 'manual_statement_cashflow_dup'",
        [],
    )
    .expect("set manual source as legacy");

    // ── cashflow ──
    // Fixture March-2026 expenses without the dup: 152.30 + 150.00 = 302.30.
    // Double-counting the shadowed -150.00 manual row would yield 452.30.
    let cashflow_json = envs(
        cargo_bin().arg("report").arg("cashflow").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("cashflow json output");
    assert!(cashflow_json.status.success());
    let cashflow: Value =
        serde_json::from_slice(&cashflow_json.stdout).expect("valid cashflow json");
    let march = cashflow
        .as_array()
        .expect("cashflow array")
        .iter()
        .find(|m| m["month_ref"].as_str() == Some("2026-03"))
        .expect("March 2026 row");
    assert_eq!(
        march["expenses"].as_str().expect("expenses string"),
        "302.30",
        "cashflow must ignore deduped manual statement row"
    );

    // ── card summary (credit card only) ──
    // Only the Pluggy -150.00 charge belongs to shared_credit; with dedup the
    // legacy shadow drops, so total_charges must remain 150.00 (not 300.00).
    let card_json = envs(
        cargo_bin()
            .arg("report")
            .arg("card-summary")
            .arg("--month")
            .arg("2026-03")
            .arg("--raw"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("card-summary raw output");
    assert!(card_json.status.success());
    let cards: Value = serde_json::from_slice(&card_json.stdout).expect("valid cards json");
    let card_row = cards
        .as_array()
        .expect("cards array")
        .iter()
        .find(|c| c["account_id"].as_str() == Some("shared_credit"))
        .expect("shared_credit row in card-summary");
    let total_charges: f64 = card_row["total_charges"]
        .as_str()
        .expect("total_charges decimal string")
        .parse()
        .expect("decimal parse");
    assert!(
        (total_charges - 150.0).abs() < 0.01,
        "card-summary must ignore deduped manual statement row; got {total_charges}"
    );
}

#[test]
fn report_ofx_consistency_compares_transactions_row_by_row() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    for (tx_id, date, description, amount) in [
        ("tx-ofx-1", "2026-05-01", "Padaria Centro", "-20.00"),
        ("tx-only-db", "2026-05-02", "Transação Extra", "-15.50"),
    ] {
        envs(
            cargo_bin()
                .arg("tx")
                .arg("upsert-manual")
                .arg("--transaction-id")
                .arg(tx_id)
                .arg("--date")
                .arg(date)
                .arg("--description")
                .arg(description)
                .arg(format!("--amount={amount}"))
                .arg("--payment-status")
                .arg("posted"),
            &config_dir,
            &data_dir,
        )
        .assert()
        .success();
    }

    let ofx_file = temp.path().join("check.ofx");
    write_file(
        &ofx_file,
        r#"OFXHEADER:100
DATA:OFXSGML
VERSION:102
<OFX>
<CREDITCARDMSGSRSV1>
<CCSTMTTRNRS>
<CCSTMTRS>
<CCACCTFROM>
<ACCTID>fixture-card-1</ACCTID>
</CCACCTFROM>
<BANKTRANLIST>
<DTSTART>20260501000000[-3:BRT]</DTSTART>
<DTEND>20260502000000[-3:BRT]</DTEND>
<STMTTRN>
<TRNTYPE>DEBIT</TRNTYPE>
<DTPOSTED>20260501000000[-3:BRT]</DTPOSTED>
<TRNAMT>-20.00</TRNAMT>
<FITID>fit-1</FITID>
<MEMO>Padaria Centro</MEMO>
</STMTTRN>
<STMTTRN>
<TRNTYPE>DEBIT</TRNTYPE>
<DTPOSTED>20260502</DTPOSTED>
<TRNAMT>-40.00</TRNAMT>
<FITID>fit-2</FITID>
<MEMO>Supermercado Delta</MEMO>
</STMTTRN>
</BANKTRANLIST>
</CCSTMTRS>
</CCSTMTTRNRS>
</CREDITCARDMSGSRSV1>
</OFX>
"#,
    );

    envs(
        cargo_bin()
            .arg("report")
            .arg("ofx-consistency")
            .arg("--ofx")
            .arg(&ofx_file),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("OFX consistency check"))
    .stdout(predicate::str::contains("faltando na base: 1"))
    .stdout(predicate::str::contains("sobrando na base: 1"))
    .stdout(predicate::str::contains("Padaria Centro"))
    .stdout(predicate::str::contains("Supermercado Delta"));

    let json_output = envs(
        cargo_bin()
            .arg("report")
            .arg("ofx-consistency")
            .arg("--ofx")
            .arg(&ofx_file)
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("ofx-consistency json output");
    assert!(json_output.status.success());
    let payload: Value = serde_json::from_slice(&json_output.stdout).expect("valid json payload");
    assert_eq!(payload["ofxTransactions"], 2);
    assert_eq!(payload["matched"], 1);
    assert_eq!(payload["missingInFinance"], 1);
    assert_eq!(payload["extraInFinance"], 1);
    assert_eq!(payload["consistent"], false);
}

#[test]
fn sync_json_summary_counts_only_new_transactions() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    let first = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("first sync output");
    assert!(first.status.success());
    let first_json: Value = serde_json::from_slice(&first.stdout).expect("first sync json");
    assert_eq!(first_json["newTransactionsCount"], 4);
    assert_eq!(first_json["needsContextCount"], 0);
    assert_eq!(first_json["newTransactions"].as_array().unwrap().len(), 4);
    let first_items = first_json["newTransactions"]
        .as_array()
        .expect("new transactions array");
    let first_tx = first_items
        .iter()
        .find(|item| item["transactionId"] == "pluggy-fixture-001")
        .expect("fixture tx in summary");
    assert_eq!(first_tx["txType"], "debit");
    assert_eq!(first_tx["categorySource"], "pluggy");
    assert_eq!(first_tx["dayOfWeek"], "friday");
    assert_eq!(first_tx["accountLabel"], "Primary Checking");
    assert_eq!(
        first_tx["metadataJson"]["pluggy_account_id"],
        "fixture-checking"
    );

    let second = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("second sync output");
    assert!(second.status.success());
    let second_json: Value = serde_json::from_slice(&second.stdout).expect("second sync json");
    assert_eq!(second_json["newTransactionsCount"], 0);
    assert_eq!(second_json["needsContextCount"], 0);
    assert_eq!(second_json["newTransactions"].as_array().unwrap().len(), 0);
}

#[test]
fn sync_json_summary_includes_pending_metadata_fields() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\n",
    );

    let fixture_path = temp.path().join("pluggy_fixture_uncategorized.json");
    write_file(
        &fixture_path,
        r#"{
  "accounts": [
    {
      "id": "fixture-checking",
      "itemId": "item-1",
      "name": "Primary Checking",
      "type": "checking",
      "status": "ACTIVE",
      "updatedAt": "2026-03-15T12:00:00.000Z"
    }
  ],
  "transactions": [
    {
      "id": "uncat-fixture-001",
      "accountId": "fixture-checking",
      "date": "2026-03-16",
      "description": "Compra sem categoria",
      "amount": -19.90,
      "type": "debit",
      "status": "POSTED",
      "createdAt": "2026-03-16T12:00:00.000Z",
      "updatedAt": "2026-03-16T12:00:00.000Z"
    }
  ]
}"#,
    );

    let output = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("pending sync output");
    assert!(output.status.success());

    let summary: Value = serde_json::from_slice(&output.stdout).expect("pending summary json");
    assert_eq!(summary["needsContextCount"], 1);
    let pending = summary["needsContext"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("pending row");
    assert_eq!(pending["transactionId"], "uncat-fixture-001");
    assert_eq!(pending["txType"], "debit");
    assert_eq!(pending["dayOfWeek"], "monday");
    assert_eq!(pending["accountLabel"], "Primary Checking");
    assert_eq!(
        pending["metadataJson"]["pluggy_account_id"],
        "fixture-checking"
    );
}

#[test]
fn sync_notify_summary_outputs_human_readable_message() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--notify-summary"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("💰 *4 novas transações*"))
    .stdout(predicate::str::contains("Supermercado Angeloni"))
    .stdout(predicate::str::contains("Saldo em conta"))
    .stdout(predicate::str::contains("Despesa real nova"))
    .stdout(predicate::str::contains("Pluggy sincronizado"))
    .stdout(predicate::str::contains("*Top categorias*"));
}

#[test]
fn sync_rejects_multiple_summary_output_modes() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--json-summary")
            .arg("--notify-summary"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .failure()
    .stderr(predicate::str::contains(
        "Use apenas uma saída de resumo: --json-summary ou --notify-summary",
    ));
}

#[test]
fn sync_json_summary_survives_missing_effective_view() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "shared_credit", "pluggyAccountId": "fixture-credit" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,fixture-credit,item-2,3,10\n",
    );

    let db_path = data_dir.join("finance-os.local.db");
    {
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute("DROP VIEW v_transactions_effective", [])
            .expect("drop effective view");
    }

    let fixture_path = temp.path().join("pluggy_fixture_uncategorized.json");
    write_file(
        &fixture_path,
        r#"{
  "accounts": [
    {
      "id": "fixture-checking",
      "itemId": "item-1",
      "name": "Primary Checking",
      "type": "checking",
      "status": "ACTIVE",
      "updatedAt": "2026-03-15T12:00:00.000Z"
    },
    {
      "id": "fixture-credit",
      "itemId": "item-2",
      "name": "Shared Credit Card",
      "type": "credit",
      "status": "ACTIVE",
      "updatedAt": "2026-03-15T12:00:00.000Z"
    }
  ],
  "transactions": [
    {
      "id": "uncat-fixture-001",
      "accountId": "fixture-checking",
      "date": "2026-03-16",
      "description": "Compra sem categoria",
      "amount": -19.90,
      "type": "debit",
      "status": "POSTED",
      "createdAt": "2026-03-16T12:00:00.000Z",
      "updatedAt": "2026-03-16T12:00:00.000Z"
    },
    {
      "id": "pluggy-fixture-aux-002",
      "accountId": "fixture-credit",
      "date": "2026-03-16",
      "description": "Compra categorizada",
      "amount": -11.50,
      "type": "debit",
      "status": "POSTED",
      "category": "alimentacao:restaurantes",
      "createdAt": "2026-03-16T12:00:00.000Z",
      "updatedAt": "2026-03-16T12:00:00.000Z"
    }
  ]
}"#,
    );
    let output = envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31")
            .arg("--json-summary"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("partial sync output");
    assert!(output.status.success());

    let summary: Value = serde_json::from_slice(&output.stdout).expect("partial summary json");
    assert_eq!(summary["summaryStatus"], "partial");
    assert_eq!(summary["newTransactionsCount"], 2);
    assert_eq!(summary["needsContextCount"], 1);
    assert_eq!(summary["needsContextReturnedCount"], 1);
    let pending = summary["needsContext"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("pending row");
    assert_eq!(pending["transactionId"], "uncat-fixture-001");
    assert_eq!(pending["description"], "Compra sem categoria");
    let warnings = summary["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1);
    assert!(
        warnings[0]
            .as_str()
            .unwrap_or_default()
            .contains("needs_context_fallback_sync_only"),
        "warning should explain degraded summary: {warnings:?}"
    );

    let conn = Connection::open(&db_path).expect("reopen db");
    let tx_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM transactions", [], |row| row.get(0))
        .expect("count transactions");
    assert_eq!(tx_count, 2);
}

#[test]
fn sync_rebinds_fixture_account_by_item_id() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "stale-checking", "pluggyItemId": "item-1" },
    { "id": "shared_credit", "pluggyAccountId": "stale-credit", "pluggyItemId": "item-2" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,stale-checking,item-1,,\nshared_credit,secondary,credit,fintech,Shared Credit Card,stale-credit,item-2,3,10\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- transactions: 4"));

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let rebound_checking: String = conn
        .query_row(
            "SELECT pluggy_account_id FROM accounts WHERE account_id = 'primary_checking'",
            [],
            |row| row.get(0),
        )
        .expect("checking rebind");
    let rebound_credit: String = conn
        .query_row(
            "SELECT pluggy_account_id FROM accounts WHERE account_id = 'shared_credit'",
            [],
            |row| row.get(0),
        )
        .expect("credit rebind");

    assert_eq!(rebound_checking, "fixture-checking");
    assert_eq!(rebound_credit, "fixture-credit");
}

#[test]
fn rule_and_context_inspection_commands_work() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("rule")
            .arg("upsert")
            .arg("--rule-id")
            .arg("mercado_rule")
            .arg("--body")
            .arg("if description contains mercado then category alimentacao:mercado"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("rule")
            .arg("upsert")
            .arg("--rule-id")
            .arg("disabled_rule")
            .arg("--body")
            .arg("if description contains farmacia then category saude:farmacia")
            .arg("--status")
            .arg("disabled"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let rule_list = envs(
        cargo_bin().arg("rule").arg("list").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("rule list output");
    assert!(rule_list.status.success());
    let listed_rules: Value = serde_json::from_slice(&rule_list.stdout).expect("rule list json");
    let listed_rules = listed_rules.as_array().expect("listed rules array");
    assert_eq!(listed_rules.len(), 1);
    assert_eq!(listed_rules[0]["rule_id"], "mercado_rule");

    let inspected = envs(
        cargo_bin()
            .arg("rule")
            .arg("inspect")
            .arg("--rule-id")
            .arg("disabled_rule")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("rule inspect output");
    assert!(inspected.status.success());
    let inspected_rule: Value =
        serde_json::from_slice(&inspected.stdout).expect("rule inspect json");
    assert_eq!(inspected_rule["status"], "disabled");

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context")
            .arg("--transaction-id")
            .arg("pluggy-fixture-001")
            .arg("--context")
            .arg("compras-do-mes"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let contexts = envs(
        cargo_bin().arg("tx").arg("list-context").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .output()
    .expect("list context output");
    assert!(contexts.status.success());
    let contexts_json: Value = serde_json::from_slice(&contexts.stdout).expect("list context json");
    let contexts_rows = contexts_json.as_array().expect("contexts array");
    let context_row = contexts_rows
        .iter()
        .find(|row| row["transaction_id"] == "pluggy-fixture-001")
        .expect("context row");
    assert_eq!(context_row["context"], "compras-do-mes");
    assert_eq!(context_row["account_label"], "Primary Checking");
}

#[test]
fn sync_applies_db_rules_without_hardcoded_personal_logic() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("rule")
            .arg("upsert")
            .arg("--rule-id")
            .arg("bill_payment_rule")
            .arg("--body")
            .arg(
                "if description contains \"pagamento de fatura\" then category credit-card-payment",
            ),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Dates relative to today so the test stays stable over time.
    let today = Utc::now().date_naive();
    let tx_date = today - Duration::days(5);
    let tx_date_str = tx_date.format("%Y-%m-%d").to_string();
    let tx_ts_str = format!("{tx_date_str}T12:00:00.000Z");
    let sync_start = (today - Duration::days(30)).format("%Y-%m-%d").to_string();
    let month_str = tx_date.format("%Y-%m").to_string();
    let to_str = today.format("%Y-%m-%d").to_string();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        &format!(
            r#"{{
  "syncStartDate": "{sync_start}",
  "accounts": [
    {{ "id": "primary_checking", "pluggyAccountId": "fixture-checking" }}
  ]
}}"#
        ),
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\n",
    );

    let fixture_path = temp.path().join("pluggy_fixture_rule.json");
    write_file(
        &fixture_path,
        &format!(
            r#"{{
  "accounts": [
    {{
      "id": "fixture-checking",
      "itemId": "item-1",
      "name": "Primary Checking",
      "type": "checking",
      "status": "ACTIVE",
      "updatedAt": "{tx_ts_str}"
    }}
  ],
  "transactions": [
    {{
      "id": "rule-fixture-001",
      "accountId": "fixture-checking",
      "date": "{tx_date_str}",
      "description": "Pagamento de fatura Visa",
      "amount": -500.00,
      "status": "POSTED",
      "createdAt": "{tx_ts_str}",
      "updatedAt": "{tx_ts_str}"
    }}
  ]
}}"#
        ),
    );

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg(&to_str),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- transactions: 1"));

    // The transaction is classified as `credit-card-payment` (internal category),
    // so the default human-friendly summary hides it from the grouped view.
    // Use --raw to assert the JSON-shaped data still includes it.
    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("120")
            .arg("--raw"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Pagamento de fatura Visa"))
    .stdout(predicate::str::contains("\"credit-card-payment\""));

    envs(
        cargo_bin()
            .arg("report")
            .arg("monthly-spend")
            .arg("--month")
            .arg(&month_str),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- linhas: 0"));

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let category_id: String = conn
        .query_row(
            "SELECT category_id FROM transactions WHERE transaction_id = 'rule-fixture-001'",
            [],
            |row| row.get(0),
        )
        .expect("category from db rule");
    let category_source: String = conn
        .query_row(
            "SELECT category_source FROM transactions WHERE transaction_id = 'rule-fixture-001'",
            [],
            |row| row.get(0),
        )
        .expect("category source from db rule");

    assert_eq!(category_id, "credit-card-payment");
    assert_eq!(category_source, "rule");
}

#[test]
fn sync_fails_when_pluggy_item_id_diverges_between_sources() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking", "pluggyItemId": "item-from-config" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-from-csv,,\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .failure()
    .stderr(predicate::str::contains("diverge"));
}

#[test]
fn sync_fails_when_two_bindings_resolve_to_same_pluggy_account() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Two bindings both targeting the same fixture account id: collision must be detected.
    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-01",
  "accounts": [
    { "id": "primary_checking", "pluggyAccountId": "fixture-checking" },
    { "id": "duplicate_checking", "pluggyAccountId": "fixture-checking" }
  ]
}"#,
    );

    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nprimary_checking,primary,checking,fintech,Primary Checking,fixture-checking,item-1,,\nduplicate_checking,primary,checking,fintech,Duplicate,fixture-checking,item-1,,\n",
    );

    let fixture_path = repo_root().join("examples/pluggy_fixture.json");

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture_path)
            .arg("--to")
            .arg("2026-03-31"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .failure()
    .stderr(predicate::str::contains("Colisão"));
}

#[test]
fn rule_list_rejects_invalid_status_value() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("rule")
            .arg("list")
            .arg("--status")
            .arg("nope"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .failure();
}

#[test]
fn tx_list_context_text_output_includes_transaction_id() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context")
            .arg("--transaction-id")
            .arg("pluggy-fixture-001")
            .arg("--context")
            .arg("compras-do-mes"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("tx").arg("list-context"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("pluggy-fixture-001"));
}

// ─── Part A: account_snapshots ──────────────────────────────────────────────

/// Syncing twice on the same day must produce exactly one snapshot per account
/// (idempotency via UNIQUE idempotency_key with INSERT OR IGNORE).
#[test]
fn account_snapshot_idempotent_same_day() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);

    // Sync twice — snapshots must not duplicate.
    seed_fixture_sync(&temp, &config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM account_snapshots", [], |row| {
            row.get(0)
        })
        .expect("count snapshots");
    // Two accounts, synced twice in the same run → still 2 (idempotent).
    assert_eq!(count, 2, "expected 1 snapshot per account, not duplicates");
}

/// Each snapshot must record the balance from the fixture at sync time.
#[test]
fn account_snapshot_records_balance() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");

    // Checking account fixture balance is 1342.44.
    let checking_balance: Option<String> = conn
        .query_row(
            "SELECT balance FROM account_snapshots WHERE account_id = 'primary_checking'",
            [],
            |row| row.get(0),
        )
        .expect("query checking balance");
    assert!(
        checking_balance.is_some(),
        "checking snapshot must have a balance"
    );
    let balance_str = checking_balance.unwrap();
    assert!(
        balance_str.contains("1342"),
        "balance should be ~1342.44, got {balance_str}"
    );
}

/// Snapshot source column must be 'pluggy'.
#[test]
fn account_snapshot_source_is_pluggy() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");

    let distinct_sources: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT source FROM account_snapshots")
            .expect("prepare");
        stmt.query_map([], |row| row.get(0))
            .expect("query")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("collect")
    };
    assert_eq!(distinct_sources, vec!["pluggy"]);
}

// ─── Part B: tx find ────────────────────────────────────────────────────────

/// `tx find` with no matching description returns empty results without error.
#[test]
fn tx_find_no_match_returns_empty() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("find")
            .arg("--query")
            .arg("xyznonexistent"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("- linhas: 0"));
}

/// `tx find` with a matching description returns that transaction.
#[test]
fn tx_find_single_match() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // "Supermercado Angeloni" is in the fixture.
    envs(
        cargo_bin()
            .arg("tx")
            .arg("find")
            .arg("--query")
            .arg("Angeloni"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("pluggy-fixture-001"));
}

/// `tx find` is case-insensitive.
#[test]
fn tx_find_case_insensitive() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // "angeloni" lower-case should still match "Supermercado Angeloni".
    envs(
        cargo_bin()
            .arg("tx")
            .arg("find")
            .arg("--query")
            .arg("angeloni"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("pluggy-fixture-001"));
}

/// `tx find --json` returns valid JSON array.
#[test]
fn tx_find_json_output() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let output = envs(
        cargo_bin()
            .arg("tx")
            .arg("find")
            .arg("--query")
            .arg("recebido")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(parsed.is_array(), "should be a JSON array");
}

/// `tx find` with multiple matching descriptions returns all of them.
#[test]
fn tx_find_multiple_matches() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // Add a manual transaction that also matches "compra".
    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-compra-001")
            .arg("--account-id")
            .arg("primary_checking")
            .arg("--date")
            .arg("2026-03-20")
            .arg("--description")
            .arg("Loja de Compras Alpha")
            .arg("--amount=-50.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("tx")
            .arg("upsert-manual")
            .arg("--transaction-id")
            .arg("manual-compra-002")
            .arg("--account-id")
            .arg("primary_checking")
            .arg("--date")
            .arg("2026-03-21")
            .arg("--description")
            .arg("Loja de Compras Beta")
            .arg("--amount=-75.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let output = envs(
        cargo_bin()
            .arg("tx")
            .arg("find")
            .arg("--query")
            .arg("compras")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("array");
    assert!(
        arr.len() >= 2,
        "expected at least 2 matches, got {}",
        arr.len()
    );
}

// ─── Part B: tx pending ─────────────────────────────────────────────────────

/// `tx pending` only returns transactions with context IS NULL.
#[test]
fn tx_pending_returns_only_no_context_txs() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // Assign context to one transaction so it should not appear in pending.
    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context")
            .arg("--transaction-id")
            .arg("pluggy-fixture-001")
            .arg("--context")
            .arg("compras"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let output = envs(
        cargo_bin().arg("tx").arg("pending").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("array");

    // pluggy-fixture-001 must NOT appear (it has context now).
    let has_001 = arr
        .iter()
        .any(|v| v["transaction_id"].as_str() == Some("pluggy-fixture-001"));
    assert!(
        !has_001,
        "fixture-001 should not appear in pending (has context)"
    );

    // All returned transactions must have null context.
    for tx in arr {
        let ctx = &tx["context"];
        assert!(
            ctx.is_null(),
            "pending tx should have null context, got {ctx}"
        );
    }
}

/// `tx pending` respects the --limit flag.
#[test]
fn tx_pending_respects_limit() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let output = envs(
        cargo_bin()
            .arg("tx")
            .arg("pending")
            .arg("--limit")
            .arg("1")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("array");
    assert!(arr.len() <= 1, "limit 1 should return at most 1 result");
}

// ─── Part B: tx set-context-by-desc ─────────────────────────────────────────

/// `--dry-run` must not write any changes to the database.
#[test]
fn tx_set_context_by_desc_dry_run_writes_nothing() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // Dry-run: should succeed and show what would change.
    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context-by-desc")
            .arg("--query")
            .arg("Angeloni")
            .arg("--context")
            .arg("mercado-teste")
            .arg("--dry-run"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("pluggy-fixture-001"));

    // Verify nothing was written to DB.
    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let ctx: Option<String> = conn
        .query_row(
            "SELECT context FROM transactions WHERE transaction_id = 'pluggy-fixture-001'",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert!(ctx.is_none(), "dry-run must not write context, got {ctx:?}");
}

/// Real `set-context-by-desc` applies the context and emits audit events.
#[test]
fn tx_set_context_by_desc_real_applies_context() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context-by-desc")
            .arg("--query")
            .arg("angeloni")
            .arg("--context")
            .arg("mercado-mes"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("1 transação"));

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let ctx: Option<String> = conn
        .query_row(
            "SELECT context FROM transactions WHERE transaction_id = 'pluggy-fixture-001'",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert_eq!(ctx.as_deref(), Some("mercado-mes"));

    // Audit event must exist.
    let audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM audit_log WHERE entity_id = 'pluggy-fixture-001' AND action = 'set_context_by_desc'",
            [],
            |row| row.get(0),
        )
        .expect("audit count");
    assert!(audit_count >= 1, "audit event must be recorded");
}

/// Re-running `set-context-by-desc` with the same context is a no-op (idempotent result,
/// context stays the same, no crash).
#[test]
fn tx_set_context_by_desc_idempotent_rerun() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    for _ in 0..2 {
        envs(
            cargo_bin()
                .arg("tx")
                .arg("set-context-by-desc")
                .arg("--query")
                .arg("Angeloni")
                .arg("--context")
                .arg("mercado-idem"),
            &config_dir,
            &data_dir,
        )
        .assert()
        .success();
    }

    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(db_path).expect("open db");
    let ctx: Option<String> = conn
        .query_row(
            "SELECT context FROM transactions WHERE transaction_id = 'pluggy-fixture-001'",
            [],
            |row| row.get(0),
        )
        .expect("query");
    assert_eq!(ctx.as_deref(), Some("mercado-idem"));
}

/// `set-context-by-desc --json` returns a JSON array of result objects.
#[test]
fn tx_set_context_by_desc_json_output() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    setup_local_auth_migrate(&config_dir, &data_dir);
    seed_fixture_sync(&temp, &config_dir, &data_dir);

    let output = envs(
        cargo_bin()
            .arg("tx")
            .arg("set-context-by-desc")
            .arg("--query")
            .arg("Angeloni")
            .arg("--context")
            .arg("mercado-json")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid JSON");
    let arr = parsed.as_array().expect("JSON array");
    assert!(!arr.is_empty(), "should return at least one result");
    assert_eq!(arr[0]["newContext"].as_str(), Some("mercado-json"));
    assert_eq!(arr[0]["transactionId"].as_str(), Some("pluggy-fixture-001"));
}

fn setup_local(temp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");
    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();
    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();
    (config_dir, data_dir)
}

#[test]
fn budget_upsert_creates_and_updates() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    // First upsert — creates
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-1")
            .arg("--amount")
            .arg("1000.00")
            .arg("--alert-threshold-pct")
            .arg("80"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Budget salvo:"))
    .stdout(predicate::str::contains("test-cat-1"));

    // List and verify
    envs(
        cargo_bin().arg("budget").arg("list").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("test-cat-1"))
    .stdout(predicate::str::contains("1000"));

    // Second upsert with same key — updates amount
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-1")
            .arg("--amount")
            .arg("1500.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // List should still show only one budget and the updated amount
    let output = envs(
        cargo_bin().arg("budget").arg("list").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let parsed: Value = serde_json::from_slice(&output).expect("valid json");
    let arr = parsed.as_array().expect("array");
    assert_eq!(
        arr.len(),
        1,
        "should have exactly one budget after two upserts"
    );
    let amount = arr[0]["amount"].as_str().expect("amount str");
    assert!(amount.contains("1500"), "amount should be updated to 1500");
}

#[test]
fn budget_list_filters_by_month() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    // Insert a default budget (no month)
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-1")
            .arg("--amount")
            .arg("500.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Insert a month-specific budget
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-2")
            .arg("--month")
            .arg("2026-04")
            .arg("--amount")
            .arg("800.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // List without filter returns both
    let output = envs(
        cargo_bin().arg("budget").arg("list").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let parsed: Value = serde_json::from_slice(&output).expect("valid json");
    assert_eq!(parsed.as_array().unwrap().len(), 2);

    // List filtered by month returns both (default + explicit for that month)
    let output = envs(
        cargo_bin()
            .arg("budget")
            .arg("list")
            .arg("--month")
            .arg("2026-04")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let parsed: Value = serde_json::from_slice(&output).expect("valid json");
    assert!(
        !parsed.as_array().unwrap().is_empty(),
        "at least the default budget should appear"
    );
}

#[test]
fn budget_status_shows_usage_pct() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    // Insert a budget for alimentacao (the fixture has grocery spend)
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("alimentacao")
            .arg("--month")
            .arg("2026-03")
            .arg("--amount")
            .arg("300.00")
            .arg("--alert-threshold-pct")
            .arg("50"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin()
            .arg("report")
            .arg("budget-status")
            .arg("--month")
            .arg("2026-03"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("alimentacao"))
    .stdout(predicate::str::contains("Orçamentos"));
}

#[test]
fn budget_status_json_is_parseable() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    seed_fixture_sync(&temp, &config_dir, &data_dir);

    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-json")
            .arg("--month")
            .arg("2026-03")
            .arg("--amount")
            .arg("100.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let output = envs(
        cargo_bin()
            .arg("report")
            .arg("budget-status")
            .arg("--month")
            .arg("2026-03")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value =
        serde_json::from_slice(&output).expect("report budget-status --json must be valid JSON");
    let arr = parsed.as_array().expect("must be an array");
    assert!(
        arr.iter().any(|row| row["category_id"] == "test-cat-json"),
        "test-cat-json should appear in budget status"
    );
    // Check required fields exist
    let row = arr
        .iter()
        .find(|r| r["category_id"] == "test-cat-json")
        .unwrap();
    assert!(row.get("budget_amount").is_some());
    assert!(row.get("actual_amount").is_some());
    assert!(row.get("usage_pct").is_some());
    assert!(row.get("projected_pct").is_some());
    assert!(row.get("alert").is_some());
}

#[test]
fn budget_default_applies_to_every_month() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    // Default budget (no month_ref)
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-default")
            .arg("--amount")
            .arg("200.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Should appear in budget-status for any queried month
    let output = envs(
        cargo_bin()
            .arg("report")
            .arg("budget-status")
            .arg("--month")
            .arg("2025-01")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid json");
    let arr = parsed.as_array().expect("array");
    assert!(
        arr.iter()
            .any(|row| row["category_id"] == "test-cat-default"),
        "default budget should appear for any queried month"
    );
}

#[test]
fn budget_explicit_month_takes_precedence_over_default() {
    let temp = TempDir::new().expect("tempdir");
    let (config_dir, data_dir) = setup_local(&temp);

    // Default budget
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-prec")
            .arg("--amount")
            .arg("200.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Month-specific override
    envs(
        cargo_bin()
            .arg("budget")
            .arg("upsert")
            .arg("--category-id")
            .arg("test-cat-prec")
            .arg("--month")
            .arg("2026-04")
            .arg("--amount")
            .arg("999.00"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    let output = envs(
        cargo_bin()
            .arg("report")
            .arg("budget-status")
            .arg("--month")
            .arg("2026-04")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let parsed: Value = serde_json::from_slice(&output).expect("valid json");
    let arr = parsed.as_array().expect("array");
    let row = arr
        .iter()
        .find(|r| r["category_id"] == "test-cat-prec")
        .expect("test-cat-prec must appear");
    let budget = row["budget_amount"].as_str().unwrap_or_default();
    assert!(
        budget.contains("999"),
        "explicit month budget (999) should take precedence over default (200), got: {budget}"
    );
}

/// Regression test for installment detection with Pluggy-style data.
///
/// Real Pluggy responses often have a normalised `description` (no "X/Y") and
/// store the installment number in `creditCardMetadata.installmentNumber` /
/// `totalInstallments`, or in `descriptionRaw`.  The fix must surface those
/// transactions in both `report installments` and `cards --installments-only`.
#[test]
fn installments_detected_from_pluggy_credit_card_metadata() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    setup_local_auth_migrate(&config_dir, &data_dir);

    // Pluggy fixture: two credit-card installment transactions.
    //   tx-inst-meta: description is clean; installment info only in
    //                 creditCardMetadata (most common Pluggy pattern).
    //   tx-inst-raw:  description is clean; installment info in descriptionRaw.
    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-04-01",
  "accounts": [
    { "id": "cc-test", "pluggyAccountId": "pluggy-cc-test" }
  ]
}"#,
    );
    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\ncc-test,primary,credit,fintech,Test Card,pluggy-cc-test,item-9,3,10\n",
    );
    let fixture = temp.path().join("inst_fixture.json");
    write_file(
        &fixture,
        r#"{
  "accounts": [
    {
      "id": "pluggy-cc-test",
      "item_id": "item-9",
      "name": "Test Card",
      "type": "credit",
      "status": "ACTIVE",
      "balance": -500.00,
      "currency_code": "BRL",
      "updated_at": "2026-04-10T00:00:00Z"
    }
  ],
  "transactions": [
    {
      "id": "tx-inst-meta",
      "accountId": "pluggy-cc-test",
      "date": "2026-04-05",
      "description": "Notebook Pro",
      "amount": -450.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-04-05T00:00:00Z",
      "updated_at": "2026-04-05T00:00:00Z",
      "creditCardMetadata": {
        "installmentNumber": 3,
        "totalInstallments": 10,
        "totalAmount": 4500.00
      }
    },
    {
      "id": "tx-inst-raw",
      "accountId": "pluggy-cc-test",
      "date": "2026-04-08",
      "description": "Amazon Marketplace",
      "amount": -200.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-04-08T00:00:00Z",
      "updated_at": "2026-04-08T00:00:00Z",
      "descriptionRaw": "Amazon Marketplace 2/6"
    }
  ]
}"#,
    );

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture)
            .arg("--to")
            .arg("2026-04-30"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // report installments must find both active chains.
    let output = envs(
        cargo_bin().arg("report").arg("installments").arg("--raw"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let chains: Value = serde_json::from_slice(&output).expect("valid json");
    let arr = chains.as_array().expect("chains array");
    assert!(
        !arr.is_empty(),
        "report installments deve retornar parcelas ativas, mas retornou vazio\nchains json: {}",
        String::from_utf8_lossy(&output)
    );

    // creditCardMetadata chain: current=3, total=10, remaining=7.
    // The JSON uses camelCase keys (baseDescription, accountId, …).
    let meta_chain = arr
        .iter()
        .find(|c| {
            c["baseDescription"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("notebook")
        })
        .unwrap_or_else(|| {
            panic!(
                "cadeia 'Notebook Pro' não encontrada\ncadeias encontradas: {}",
                serde_json::to_string_pretty(arr).unwrap()
            )
        });
    assert_eq!(meta_chain["total"], 10, "total deve ser 10");
    assert_eq!(meta_chain["current"], 3, "current deve ser 3");
    assert!(
        meta_chain["remaining"].as_u64().unwrap_or(0) > 0,
        "remaining deve ser > 0"
    );

    // descriptionRaw chain: current=2, total=6, remaining=4.
    let raw_chain = arr
        .iter()
        .find(|c| {
            c["baseDescription"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("amazon")
        })
        .expect("cadeia 'Amazon Marketplace' não encontrada");
    assert_eq!(raw_chain["total"], 6, "total deve ser 6");
    assert_eq!(raw_chain["current"], 2, "current deve ser 2");

    // cards --installments-only must surface both transactions.
    let card_output = envs(
        cargo_bin()
            .arg("report")
            .arg("cards")
            .arg("--month")
            .arg("2026-04")
            .arg("--installments-only")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    let card_json: Value = serde_json::from_slice(&card_output).expect("valid json");
    let txs = card_json["transactions"]
        .as_array()
        .expect("transactions array");
    assert_eq!(
        txs.len(),
        2,
        "cards --installments-only deve retornar as 2 transações parceladas, obteve {}",
        txs.len()
    );
}

/// Regression test: `cards --installments-only` must preserve the underlying
/// bill's payment status. The filter narrows the *displayed* transactions
/// (totals/counts/category breakdown), but should NOT shrink the total used
/// for matching against checking-account bill payments — otherwise a bill
/// that was actually paid in full would be reported as "em aberto" /
/// "ATRASADO" simply because the installment-only subtotal doesn't match
/// any payment line.
#[test]
fn cards_installments_only_preserves_paid_status() {
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    setup_local_auth_migrate(&config_dir, &data_dir);

    // closing_day=3 → April transactions on/after the 3rd belong to the
    // bill that closes 2026-05-03; close_date < today (2026-05-19+) so the
    // bill is treated as closed and payment matching kicks in.
    let pluggy_config = temp.path().join("pluggy-config.json");
    write_file(
        &pluggy_config,
        r#"{
  "syncStartDate": "2026-03-15",
  "accounts": [
    { "id": "chk", "pluggyAccountId": "pluggy-chk" },
    { "id": "cc", "pluggyAccountId": "pluggy-cc" }
  ]
}"#,
    );
    let accounts_csv = temp.path().join("contas.csv");
    write_file(
        &accounts_csv,
        "id,owner,type,bank,label,pluggy_account_id,pluggy_item_id,billing_closing_day,billing_due_day\nchk,primary,checking,fintech,Checking,pluggy-chk,item-chk,,\ncc,primary,credit,fintech,Card,pluggy-cc,item-cc,3,10\n",
    );
    let fixture = temp.path().join("paid_with_installments.json");
    write_file(
        &fixture,
        r#"{
  "accounts": [
    {
      "id": "pluggy-chk",
      "item_id": "item-chk",
      "name": "Checking",
      "type": "checking",
      "status": "ACTIVE",
      "balance": 5000.00,
      "currency_code": "BRL",
      "updated_at": "2026-05-10T00:00:00Z"
    },
    {
      "id": "pluggy-cc",
      "item_id": "item-cc",
      "name": "Card",
      "type": "credit",
      "status": "ACTIVE",
      "balance": -1000.00,
      "currency_code": "BRL",
      "updated_at": "2026-05-10T00:00:00Z"
    }
  ],
  "transactions": [
    {
      "id": "cc-inst-1",
      "accountId": "pluggy-cc",
      "date": "2026-04-05",
      "description": "Notebook",
      "amount": -200.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-04-05T00:00:00Z",
      "updated_at": "2026-04-05T00:00:00Z",
      "creditCardMetadata": { "installmentNumber": 1, "totalInstallments": 5, "totalAmount": 1000.00 }
    },
    {
      "id": "cc-inst-2",
      "accountId": "pluggy-cc",
      "date": "2026-04-08",
      "description": "Geladeira",
      "amount": -200.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-04-08T00:00:00Z",
      "updated_at": "2026-04-08T00:00:00Z",
      "creditCardMetadata": { "installmentNumber": 2, "totalInstallments": 10, "totalAmount": 2000.00 }
    },
    {
      "id": "cc-plain",
      "accountId": "pluggy-cc",
      "date": "2026-04-15",
      "description": "Supermercado",
      "amount": -600.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-04-15T00:00:00Z",
      "updated_at": "2026-04-15T00:00:00Z"
    },
    {
      "id": "pay-1",
      "accountId": "pluggy-chk",
      "date": "2026-05-10",
      "description": "Pagamento de fatura cartao",
      "amount": -1000.00,
      "type": "debit",
      "status": "posted",
      "created_at": "2026-05-10T00:00:00Z",
      "updated_at": "2026-05-10T00:00:00Z"
    }
  ]
}"#,
    );

    envs(
        cargo_bin()
            .arg("sync")
            .arg("pluggy")
            .arg("--pluggy-config")
            .arg(&pluggy_config)
            .arg("--accounts-csv")
            .arg(&accounts_csv)
            .arg("--fixture")
            .arg(&fixture)
            .arg("--to")
            .arg("2026-05-15"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Pluggy sync rebuilds account metadata from the API payload and only
    // *preserves* an existing billing_closing_day across re-syncs — it does
    // not import the value from the accounts CSV on first sync. Inject it
    // directly so the bill clustering uses the synthetic close date that
    // makes target_month=2026-05 resolve to a closed bill with a payment.
    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(&db_path).expect("open sqlite");
    conn.execute(
        "UPDATE accounts SET metadata_json = json_set(metadata_json, '$.billing_closing_day', '3') WHERE account_id = 'cc'",
        [],
    )
    .expect("inject billing_closing_day");
    drop(conn);

    // Baseline: without the filter, the bill should be flagged as paid.
    let baseline = envs(
        cargo_bin()
            .arg("report")
            .arg("cards")
            .arg("--month")
            .arg("2026-05")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let baseline_json: Value = serde_json::from_slice(&baseline).expect("valid json");
    let acct = &baseline_json["summary"]["accounts"][0];
    assert_eq!(
        acct["status"]["state"],
        "paid",
        "baseline (sem filtro) deve mostrar a fatura como paga, summary: {}",
        serde_json::to_string_pretty(&baseline_json["summary"]).unwrap()
    );

    // With --installments-only: the displayed totals shrink, but the
    // status must still come from the full bill — so "paid" not "open".
    let filtered = envs(
        cargo_bin()
            .arg("report")
            .arg("cards")
            .arg("--month")
            .arg("2026-05")
            .arg("--installments-only")
            .arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let filtered_json: Value = serde_json::from_slice(&filtered).expect("valid json");
    let acct = &filtered_json["summary"]["accounts"][0];
    assert_eq!(
        acct["status"]["state"], "paid",
        "fatura paga não deve aparecer como 'open'/'overdue' só porque o filtro reduziu o total exibido, summary: {}",
        serde_json::to_string_pretty(&filtered_json["summary"]).unwrap()
    );
    assert_eq!(
        acct["transaction_count"], 2,
        "transaction_count deve refletir só as parceladas, obteve {}",
        acct["transaction_count"]
    );
    let txs = filtered_json["transactions"]
        .as_array()
        .expect("transactions array");
    assert_eq!(
        txs.len(),
        2,
        "transactions deve conter só as 2 parceladas, obteve {}",
        txs.len()
    );
}

#[test]
fn amount_cents_exact_integer_sum_regression() {
    // ADR-0003: verify that amount_cents produces exact integer sums
    // and that views no longer exhibit floating-point drift.
    let temp = TempDir::new().expect("tempdir");
    let config_dir = temp.path().join("config");
    let data_dir = temp.path().join("data");

    envs(
        cargo_bin()
            .arg("auth")
            .arg("setup")
            .arg("--backend")
            .arg("local")
            .arg("--actor-id")
            .arg("test-actor"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    envs(
        cargo_bin().arg("admin").arg("migrate"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success();

    // Insert transactions with edge-case cent values that would expose
    // floating-point drift under CAST(amount AS REAL) aggregation.
    let txs = [
        ("prec-001", "2026-05-01", "Um centavo", "-0.01"),
        ("prec-002", "2026-05-02", "Dois centavos", "-0.02"),
        ("prec-003", "2026-05-03", "Tres centavos", "-0.03"),
        ("prec-004", "2026-05-04", "Mil duzentos", "-1234.56"),
        ("prec-005", "2026-05-05", "Salario", "5000.00"),
        ("prec-006", "2026-05-06", "Cashback troco", "1.99"),
    ];

    for (id, date, desc, amount) in &txs {
        envs(
            cargo_bin()
                .arg("tx")
                .arg("upsert-manual")
                .arg("--transaction-id")
                .arg(id)
                .arg("--date")
                .arg(date)
                .arg("--description")
                .arg(desc)
                .arg(format!("--amount={amount}")),
            &config_dir,
            &data_dir,
        )
        .assert()
        .success();
    }

    // Verify amount_cents column directly via SQLite.
    let db_path = data_dir.join("finance-os.local.db");
    let conn = Connection::open(&db_path).expect("open db");

    // Every row must have a non-NULL amount_cents that matches ROUND(amount * 100).
    let mismatch_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions
             WHERE amount_cents IS NULL
                OR amount_cents != CAST(ROUND(CAST(amount AS REAL) * 100) AS INTEGER)",
            [],
            |row| row.get(0),
        )
        .expect("count mismatches");
    assert_eq!(mismatch_count, 0, "amount_cents mismatch detected");

    // Verify exact view aggregation: v_cashflow income.
    let cashflow_json = envs(
        cargo_bin().arg("report").arg("cashflow").arg("--json"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();

    // Cashflow report returns a JSON array of month objects.
    let cf: Value = serde_json::from_slice(&cashflow_json).expect("valid cashflow json");
    let months = cf.as_array().expect("cashflow is an array");
    let may = months
        .iter()
        .find(|m| m["month_ref"].as_str() == Some("2026-05"))
        .expect("May 2026 in cashflow");

    // Income: salary 5000.00 + uncategorized credit 1.99 = 5001.99.
    // JSON serialization of Decimal uses exact string representation.
    assert_eq!(
        may["income"].as_str().expect("income string"),
        "5001.99",
        "income deve ser exato, sem drift de ponto flutuante"
    );

    // Expenses: 0.01 + 0.02 + 0.03 + 1234.56 = 1234.62 (exact).
    assert_eq!(
        may["expenses"].as_str().expect("expenses string"),
        "1234.62",
        "expenses deve ser exato, sem drift de ponto flutuante"
    );

    // Net = income - expenses = 5001.99 - 1234.62 = 3767.37
    assert_eq!(
        may["net"].as_str().expect("net string"),
        "3767.37",
        "net deve ser exato"
    );

    // Also verify the v_cashflow view via direct SQLite query:
    // SUM(amount_cents) / 100.0 should match exactly.
    let (view_income, view_expenses, view_net): (String, String, String) = conn
        .query_row(
            "SELECT income, expenses, net FROM v_cashflow WHERE month_ref = '2026-05'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("v_cashflow query");

    assert_eq!(view_income, "5001.99", "v_cashflow income exato");
    assert_eq!(view_expenses, "1234.62", "v_cashflow expenses exato");
    assert_eq!(view_net, "3767.37", "v_cashflow net exato");
}
