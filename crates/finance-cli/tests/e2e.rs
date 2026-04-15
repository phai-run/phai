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

    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("31"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Supermercado Angeloni"))
    .stdout(predicate::str::contains("Pagamento recebido"));
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
    .stdout(predicate::str::contains("alimentacao-mercado"))
    .stdout(predicate::str::contains("saude:exames"))
    .stdout(predicate::str::contains("gas-stations"))
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
    .stdout(predicate::str::contains("2026-03"))
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
    .stdout(predicate::str::contains("em aberto"));

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
    let sync_start = (today - Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
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

    envs(
        cargo_bin()
            .arg("report")
            .arg("daily-pulse")
            .arg("--days")
            .arg("31"),
        &config_dir,
        &data_dir,
    )
    .assert()
    .success()
    .stdout(predicate::str::contains("Pagamento de fatura Visa"))
    .stdout(predicate::str::contains("- entradas: +R$ 0,00"))
    .stdout(predicate::str::contains("- saídas: +R$ 0,00"));

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
