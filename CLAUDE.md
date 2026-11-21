# scarlet_nexus_rust (SN) — Mapa e Regras do Projeto

## Objetivo
Port fiel e otimizado de *Scarlet Nexus* em Rust:
- **Fase 1 (Atual):** Leitura e conversão de assets + validação rodando no PC.
- **Fase 2 (Futura):** Adaptação para Carafe no Nintendo Switch (baixo consumo de RAM e assets leves).
- **Ritmo:** 1 recurso por dia, em ordem decrescente de dificuldade (do mais difícil para o mais fácil).

## Regras
1. Consulte este arquivo primeiro em vez de reler todo o repositório.
2. Mantenha ust/save-data-names e evidências originais intactos; isole código específico de plataforma.
3. Evite .unwrap() em entradas externas, use Result explícito e passe em cargo fmt --check, cargo clippy e cargo test.
