# YM3438 Browser Test

この段階では、現在成功している `ymfm-sys` の YM3438 WASM が
ブラウザで直接ロードできるかを確認します。

注意: 現在の `ymfm-sys` WASM は wasm32-wasip1 のWASI実行形式なので、
直接 instantiate できない可能性があります。それ自体が今回の重要な判定結果です。

## 手順
1. GitHub Actionsを実行
2. `ym3438-browser-test` Artifactをダウンロード
3. 展開した `index.html` をHTTP(S)サーバー経由で開く
4. 画面のログを確認
5. Startを押してWeb Audioの確認音を聞く

`file://` ではなくHTTP(S)で開いてください。
