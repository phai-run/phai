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
            .arg("120"),
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
    .stdout(predicate::str::contains("alimentacao mercado"))
    .stdout(predicate::str::contains("saude > exames"))
    .stdout(predicate::str::contains("gas stations"))
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
    .stdout(predicate::str::contains("Card closed insights 2026-03"))
    .stdout(predicate::str::contains("Recorrentes:"))
    .stdout(predicate::str::contains("Assinaturas:"))
    .stdout(predicate::str::contains("Parceladas fechadas:"))
    .stdout(predicate::str::contains("Parceladas em aberto:"))
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
    .stdout(predicate::str::contains("Novas transações detectadas (4):"))
    .stdout(predicate::str::contains("🍽️ Supermercado Angeloni"))
    .stdout(predicate::str::contains("🍽️ alimentacao mercado (pluggy)"))
    .stdout(predicate::str::contains("Pendências de contexto (0):"))
    .stdout(predicate::str::contains("Fonte: local | cli: finance"));
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
    .stdout(predicate::str::contains("Budget status 2026-03"));
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
