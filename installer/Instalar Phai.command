#!/usr/bin/env bash
#
# Instalar Phai — instalador gráfico para macOS.
#
# Dê dois cliques neste arquivo. Ele baixa o Phai, configura o app e abre a
# tela de ativação no navegador. Depois disso você só precisa anexar a sua
# chave e digitar a senha — sem terminal.
#
# (Na primeira vez o macOS pode pedir para confirmar a abertura: clique com o
# botão direito → Abrir, e confirme. Isso acontece porque o Phai é open source
# e não passa pela loja da Apple.)
#
set -euo pipefail

clear
cat <<'BANNER'

   φ  Phai

   Instalando… isso leva menos de um minuto.

BANNER

curl -fsSL https://raw.githubusercontent.com/phai-run/phai/main/install.sh \
  | bash -s -- --app

cat <<'DONE'

Pronto! A tela de ativação abriu no navegador.
Pode fechar esta janela.

DONE
