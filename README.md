# YM3438 Browser Test (diagnostic)

GitHub Pagesへ自動デプロイするYM3438実行テストです。

## 重要
`@wasmer/sdk` はブラウザで `SharedArrayBuffer` を使うため、cross-origin isolation が必要です。GitHub PagesではHTTPヘッダーを直接設定できないため、`coi-serviceworker.js` を同梱して初回アクセス時に自動再読み込みします。

1. GitHubにこのリポジトリをpush
2. Actionsの `Build and deploy YM3438 browser test` が成功するまで待つ
3. Settings → Pages でGitHub ActionsをPagesの公開元として設定
4. 表示されたPages URLを開く
5. 初回は自動再読み込みされるので待つ
6. 「YM3438テスト開始」を押す

ログに [1]〜[7] のどこまで進んだかが表示されます。
