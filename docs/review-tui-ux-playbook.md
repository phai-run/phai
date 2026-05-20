# Playbook UX — Revisão de categorias e transações

Documento de referência para tornar `tx review-human --tui` fluido ao analisar e recategorizar **centenas** de transações. Consolida padrões dos TUIs mais usados (opencode, codex, fzf, lazygit, yazi, lazydocker, tmux, etc.) aplicados ao que já existe em `crates/finance-cli/src/main.rs`.

**Comando hoje:** `finance tx review-human --kind all --limit N --tui [--sound] [--bulk identical]`

---

## 1. O que já está certo (manter)

A TUI atual já implementa várias “sacadas” dos projetos de referência:

| Padrão (referência) | Já no Finance OS | Onde |
|---------------------|------------------|------|
| Master–detail + fila (lazygit, dive) | Fila \| editor \| contexto | `draw_review_tui_body`, `draw_review_tui_queue` |
| Contexto para decisão (dive, yazi) | Idênticas, mesmo dia, similares, dica de regra | `ReviewTuiContext`, `load_review_tui_context` |
| Bulk com toggle visível (dive toggles) | `Ctrl+B` bulk idênticas, status no header/footer | `handle_review_tui_bulk_key`, `review_tui_bulk_status_line` |
| Salvar e avançar (fluxo de fila) | `Cmd+Enter` → `save_and_advance_tui_review` | `handle_review_tui_basic_key` |
| Navegar sem salvar | `Cmd+↑/↓` entre transações | `handle_review_tui_row_key` |
| Desfazer edição local | `Ctrl+U` restaura draft da linha | `reset_current_tui_draft_if_requested` |
| Categoria com busca incremental | Filtro + lista (8 itens), setas | `category_matches`, `draw_review_tui_category_suggestions` |
| Patch mínimo (só o que mudou) | `patch_against` / `changed_text` | `ReviewTuiDraft` |
| Contexto assíncrono sem travar UI | Cache + load no idle (poll 80ms) | `prepare_review_tui_context_for_input`, `context_cache` |
| Layout responsivo | ≥120 cols: 3 colunas; compacto &lt;24 linhas | `draw_review_tui_body`, `draw_review_tui_fields_compact` |
| Feedback ao salvar | Status no footer + bell opcional | `draw_review_tui_footer`, `--sound` |

**Princípio de produto (ADR-0014):** separar `raw_description` (leitura) de `merchant_name`, `description`, `purpose`, `category_id` (edição humana). A TUI respeita isso — não misturar raw nos campos editáveis.

---

## 2. Meta de experiência para “centenas de transações”

### Persona e objetivo

- **Quem:** você (ou Ford via WhatsApp) revisando pendências após import/sync.
- **Objetivo:** máximo de **decisões corretas por minuto**, com mínimo de teclas e zero medo de “salvar errado em massa”.
- **Não é:** preencher os 4 campos em toda linha — na prática, 70%+ das sessões são **só categoria** ou **categoria + merchant**.

### Métricas de sucesso (verificáveis)

1. **Tempo médio por transação “só categoria”** &lt; 5 s (teclado).
2. **≥80%** das idênticas resolvidas com bulk ON em uma ação.
3. **Zero** resultado de busca/categoria “atrasado” (padrão AsyncHandler).
4. Sessão de 200 itens sem reiniciar o processo (`--limit` alto ou paginação).
5. Usuário nunca precisa decorar atalhos: footer + which-key cobrem 95%.

### Fluxo ideal (happy path)

```
Entrada → Resumo (--summary) → Carregar fila grande (--limit 500)
    → Filtrar fila (ex.: só sem categoria, ou por merchant)
    → Para cada item:
         ver raw + contexto
         [1-9] categoria sugerida OU digitar fuzzy
         Cmd+Enter (salvar + próximo)
         se idênticas > 1: Ctrl+B antes de salvar
    → Pular incertos sem sair (s)
    → Marcar “revisado depois” opcional
```

---

## 3. Sacadas dos TUIs top 10 → regras para o Finance OS

### 3.1 Velocidade = menos campos por padrão (fzf, lazygit)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Modo “só categoria”** | Toggle `c` ou flag `--focus category`: esconde purpose/description na UI; Tab circula só merchant+categoria. |
| **Enter na categoria = salvar** | Com sugestão selecionada, `Enter` aplica categoria e chama o mesmo path de `Cmd+Enter` (lazygit: Enter confirma ação no painel ativo). |
| **Números 1–9 nas sugestões** | yazi/lazydocker: escolher categoria sem setas. Mapear `category_matches` índice 0..8 → teclas `1`..`9`. |
| **Última categoria repetida** | Após salvar, manter `last_category_id`; `=` ou `Ctrl+Y` cola na próxima linha (recategorização em lote do mesmo tipo). |

### 3.2 Fila como instrumento principal (lazygit, fzf)

| Regra | Implementação sugerida |
|-------|------------------------|
| **`--limit` default alto para TUI** | Ex.: 500 quando `--tui` e stdin é TTY; manter 10 para JSON/WhatsApp. |
| **Filtro na fila (filter, não search)** | `/` abre filtro: substring em raw, merchant, valor, data (lazygit Search vs Filter). Lista some itens que não batem. |
| **Ordenação estável** | Data desc, valor abs desc, ou “sem categoria primeiro”. |
| **Indicadores na fila** | Ícone/cor: sem cat, só merchant pendente, bulk disponível (N idênticas). Uma letra na coluna da fila. |
| **Pular sem salvar** | `s` ou `Ctrl+S` → próximo (hoje só no modo texto; **gap na TUI**). |
| **Marcar revisado / adiar** | `d` remove da fila local sem persistir (ou flag `review_deferred` futura). |

### 3.3 Bulk e padrões (dive, opencode, SKILL Ford)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Bulk idênticas (já existe)** | Manter `Ctrl+B`; destacar no header quando `identical_count > 1`. |
| **Bulk “similares” (novo)** | `Ctrl+Shift+B`: mesma `raw_description` normalizada ou mesmo merchant — alinhado a `review_tui_similar_context`. |
| **Pré-visualizar afetadas** | Já mostra até 5 em bulk ON; expandir com `o` em popup/lista. |
| **Promover a regra** | Quando `rule_hint` = padrão recorrente, `r` abre fluxo `tx rule` (ou copia comando para clipboard). |
| **Assistente (Ford)** | Mesmos campos do JSON `review-human --transaction-id`; TUI é a fonte de verdade dos atalhos documentados no SKILL. |

### 3.4 Teclado em escala (tmux, opencode, yazi)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Leader opcional** | `Ctrl+X` prefix para ações raras (`?` help, `r` regra) sem poluir footer. |
| **Which-key** | Durante prefix ou `?`, overlay com atalhos do contexto (painel fila vs campo categoria). |
| **Footer por foco** | Fila focada: `j/k` ou `↑/↓` mudam item; editor focado: atalhos atuais. Hoje foco é implícito — tornar explícito com `Tab` na fila. |
| **Paleta de comandos** | `Ctrl+P`: “ir para transação”, “filtrar sem categoria”, “ligar bulk”, “aumentar limite”. |
| **Config YAML de keybinds** | `tui.json` / seção em config — padrão opencode; defaults no código. |

### 3.5 Busca e categorias (fzf, yazi)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Fuzzy real** | Substituir `contains` em `category_matches` por **nucleo** (já dependência do projeto) — ranking por score, não ordem alfabética. |
| **Aliases** | `mercado` → `alimentacao:mercado`; tabela pequena em config ou derivada de histórico do usuário. |
| **Histórico de categorias usadas na sessão** | Subir no topo das sugestões as últimas 5 categorias aplicadas. |
| **AsyncHandler na fila** | Se filtro async no futuro: IDs monotônicos como `lazygit/pkg/tasks/async_handler.go` — nunca mostrar fila antiga após digitar rápido. |

### 3.6 Feedback e confiança (codex, lazydocker)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Diff antes de salvar (opcional)** | Linha no footer: `cat: (vazio) → alimentacao:mercado` quando patch só tem categoria. |
| **Confirmação só em bulk &gt; N** | Se bulk ON e `identical_count > 10`, `Enter` pede `y` uma vez. |
| **Progresso global** | Header: `47/312` + barra ASCII + “restam Z sem categoria no banco”. |
| **Som / sem som** | Manter `--sound`; respeitar preferência silenciosa em config. |
| **Undo último save** | `Ctrl+Z` desfaz último patch persistido (audit trail já existe em `review_human` events) — codex-level polish. |

### 3.7 Assistente conversacional (opencode, codex, SKILL)

| Regra | Implementação sugerida |
|-------|------------------------|
| **Mesma fila, dois modos** | TUI local; Ford usa `--json` com mesmos `transaction_id` e campos. |
| **Resumo antes da sessão** | `--summary` já existe; TUI deveria mostrar convite “312 pendências — Enter para começar”. |
| **Uma transação = um cartão** | Formato curto no WhatsApp espelha `draw_review_tui_readonly` (valor, data, raw, cat atual). |
| **Não inventar categoria** | TUI já exige escolha explícita; sugestões são lista fechada do store. |

---

## 4. Layout alvo (wireframe mental)

```
┌─ Finance OS  revisão  [████████░░] 84/312  │ bulk: ON │ faltam cat: 41 ─┐
├─ Fila (/ filtrar) ─┬─ Transação + campos ───┬─ Contexto ─────────────────┤
│ > 84 R$ -42 Mercado│  R$ -42,00  2026-05-01 │ idênticas: 6  bulk: ON    │
│   85 R$ -9  Uber   │  cat: alimentacao:…   │ regra: promover…          │
│   86 …             │  [merchant][desc]…    │ Mesmo dia: …               │
│                    │  Categorias 1-9      │ Similares: …               │
├────────────────────┴──────────────────────┴────────────────────────────┤
│ s pular │ Enter salvar │ 1-9 cat │ Ctrl+B bulk │ Ctrl+U desfazer fila  │
└──────────────────────────────────────────────────────────────────────────┘
```

**Prioridade visual:** olho vai **raw + valor** (readonly) → **categoria** → contexto idênticas. Purpose/description colapsados até `Tab` ou modo completo.

---

## 5. Roadmap priorizado

### P0 — maior ganho para centenas de itens ✅ concluído

1. **`s` / `Ctrl+S` pular** na TUI — `s` fora do campo Categoria (sem edição pendente); `Ctrl+S` sempre.
2. **`Enter` salva** quando campo ativo = Categoria (`confirm_review_tui_category_selection`).
3. **Teclas `1`–`9`** nas sugestões de categoria (`handle_review_tui_category_number_key`).
4. **`--limit` default 500** no `--tui` / `finance review` (`effective_review_human_limit`).
5. **nucleo** em `category_matches` + boost das últimas 5 categorias da sessão.
6. **Repetir última categoria** (`=` / `Ctrl+Y`) (`handle_review_tui_repeat_category_key`).

Atalho adicional: `finance review` abre a TUI com fila longa por default.

### P1 — fluxo profissional (3–5 dias)

7. **Filtro `/` na fila** (filter mode, não highlight-only).
8. **Modo foco só categoria** (`c` toggle).
9. **Indicadores na fila** (sem cat, N idênticas).
10. **Bulk similares** (`Ctrl+Shift+B`) usando mesma base de `review_tui_similar_context`.
11. **Header com progresso** vs total do banco (`uncategorized_count`).
12. **Which-key / `?`** gerado dos handlers (padrão lazydocker).

### P2 — polish e assistente (opcional)

13. Paleta `Ctrl+P` com ações de sessão.
14. Undo último save (`Ctrl+Z`).
15. Confirmação bulk &gt; N transações.
16. Keybinds em config file.
17. Integração `r` → criar regra a partir do hint.

---

## 6. Anti-padrões (não fazer)

- **Obrigar 4 campos** antes de avançar — mata throughput.
- **Salvar ao mudar de transação** sem `Cmd+Enter` — risco de gravar draft incompleto.
- **Lista de categoria alfabética sem ranking** — fzf/yazi ensinam que ranking é UX.
- **Recarregar contexto a cada frame** — manter cache (já feito); invalidar só após save/bulk.
- **Limite 10 em TUI** — adequado para WhatsApp, não para sessão de revisão local.
- **Inventar categoria fora da taxonomia** — quebra relatórios e Ford SKILL.

---

## 7. Checklist para o assistente (Ford / OpenClaw)

Ao guiar revisão fora da TUI, espelhar o que a TUI faz bem:

1. `tx review-human --summary --json` → contar pendências, convidar.
2. Carregar lote: `--kind all --limit 20 --json` (WhatsApp) ou sugerir `--tui --limit 500` (terminal).
3. Uma transação por mensagem: valor, data, raw, cat atual, contagem de idênticas.
4. Aceitar atalhos naturais: “pula”, “igual anterior”, “educação material escolar” → `categoria:sub`.
5. Se “todas iguais” → `--bulk identical`.
6. Nunca preencher categoria sem confiança; campos humanos parciais são OK.
7. Após lote, ` --summary` de novo para fechar o loop.

---

## 8. Referências de código (Finance OS)

| Área | Funções / tipos |
|------|-----------------|
| Loop principal | `tx_review_human_tui` |
| Input | `handle_review_tui_*_key` |
| Desenho | `draw_review_tui_*` |
| Dados | `ReviewTuiDraft`, `ReviewTuiContext`, `review_human_rows` |
| Bulk | `ReviewHumanBulk`, `apply_human_review_with_bulk` |
| Categorias | `category_matches`, `store.internal_categories()` |
| CLI args | `ReviewHumanArgs` (`--tui`, `--limit`, `--bulk`) |

---

## 9. Referência externa (TUIs analisados)

Consolidado em `~/awesome-tuis-top10-analise-ux.md` (análise dos repositórios com mais estrelas da lista awesome-tuis).

**Padrões mais transferíveis para este produto:**

- **lazygit:** AsyncHandler, search vs filter, modos de painel.
- **fzf:** fuzzy ranking, preview debounced, DSL de ações.
- **yazi:** which-key para chords, abort de preview/job obsoleto.
- **lazydocker:** menu `?` auto-gerado, teclas numéricas para painéis.
- **opencode:** paleta + which-key + keymap declarativo.
- **dive:** toggles de bulk visíveis, master–detail por eventos.

---

*Última atualização: maio/2026 — alinhado ao código em `main.rs` (review TUI).*
