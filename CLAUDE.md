# scarlet_nexus_rust (SN) — Mapa e Regras do Projeto

## Objetivo
Port fiel e otimizado de *Scarlet Nexus* em Rust:
- **Fase 1 (Atual):** Leitura e conversão de assets + validação rodando no PC.
- **Fase 2 (Futura):** Adaptação para Carafe no Nintendo Switch (baixo consumo de RAM e assets leves).
- **Ritmo:** 1 recurso por dia, em ordem decrescente de dificuldade (do mais difícil para o mais fácil).

## Regras
1. Consulte este arquivo primeiro em vez de reler todo o repositório.
2. Mantenha `rust/save-data-names` e evidências originais intactos; isole código específico de plataforma.
3. Evite .unwrap() em entradas externas, use Result explícito e passe em cargo fmt --check, cargo clippy e cargo test.

## Memória e Estado Atual
- **Dia 1 concluído — conversão e otimização de malhas e texturas para GLB leve:**
  - Parsers, conversores e exportador modular produzem glTF 2.0 JSON + BIN alinhado e contêiner GLB com malha, materiais e texturas PNG, incluindo teste ponta a ponta validado pela crate `gltf`.
  - Limites agregados de vértices, índices e bytes de textura são aplicados por asset; o parser PSK compartilha o total de vértices entre materiais.
  - O material fallback é neutro e não associa texturas automaticamente; os orçamentos agregados de textura e do BIN são verificados antes das alocações e cópias dos buffers, mantendo a validação final do contêiner.
  - Validação final: **51 testes passando** (50 unitários e 1 de integração), `cargo fmt --check` e `cargo clippy --all-targets` limpos.
- **Foco do Dia 2:** reduzir cópias intermediárias e picos de memória na leitura e decodificação de texturas e chunks; ampliar os limites agregados de memória/bytes por asset; refinar o perfil `switch` e o isolamento de I/O; inspecionar visualmente assets no Blender com o add-on de *Scarlet Nexus*.
- **Graphify MCP:** ativo, com 61 nós e 79 arestas.
- **Repositório:** [TEUSMURTA/SN](https://github.com/TEUSMURTA/SN), com o Dia 1 integrado na branch `main` a partir de `feat/dia-1-glb-otimizado`.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- When graphify-out/graph.json exists, run `graphify query "<question>"` before your first grep, search or source-file read for a codebase question, and put this rule in every subagent prompt that explores code. Read raw files only for lines the graph or an earlier read already located. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. When the answer is under a Markdown heading, `graphify section "<heading>"` prints that section's text, so the .md need not be opened. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
- A committed graphify-out/graph.json is rewritten whole on every build, so a large diff to it in a pull request is expected. Do not flag it as churn or suggest skipping the graph rebuild or its hooks.
