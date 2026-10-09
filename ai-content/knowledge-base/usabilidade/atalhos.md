# Atalhos

Fonte única: `src/commands.rs` (`SHORTCUTS` e `lookup`). O editor consulta esse
mapa antes das teclas próprias, então os atalhos da aplicação vencem quando ele
está focado. Fora do editor, só disparam se nenhum widget capturou a tecla.

| Tecla | Ação |
| --- | --- |
| Ctrl+Enter | Executar a seleção ou, sem seleção, a instrução sob o cursor |
| Ctrl+Shift+Enter | Executar o script inteiro |
| Ctrl+S | Salvar o arquivo SQL da aba |
| Ctrl+. | Cancelar a consulta em andamento |
| Ctrl+P | Seletor rápido (tabelas/views/arquivos do workspace ativo) |
| Ctrl+Tab / Ctrl+Shift+Tab | Próximo/anterior workspace (circular) |
| Ctrl+W | Fechar aba (o Scratch não fecha) |
| F5 | Recarregar metadados (e a lista de arquivos) |
| Tab (no editor) | Insere 4 espaços |

## Conflitos documentados

- **Ctrl+S:** a spec (§6.2) reserva o atalho para preparar alterações da grade. Hoje
  ele salva o arquivo SQL. Quando a edição de dados existir: com alterações
  pendentes na grade, Ctrl+S abre o popup; sem elas, salva o arquivo.
- **Ctrl+Shift+S:** reservado para executar alterações direto (ainda não
  implementado).
- **Ctrl+Tab** troca workspace, e não aba, por decisão da spec (§9).

## Mouse na grade

| Ação | Efeito |
| --- | --- |
| Clique no cabeçalho | Ordena (ASC → DESC → sem ordenação) e reexecuta |
| Botão do meio no cabeçalho | Oculta a coluna e a remove do SELECT, quando seguro |
| Arrastar cabeçalho e soltar sobre outro | Move a coluna e reordena o SELECT, quando seguro |
| Botão "Restaurar colunas" | Mostra de novo as colunas ocultas e restaura o SQL |
