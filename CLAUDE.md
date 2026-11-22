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
