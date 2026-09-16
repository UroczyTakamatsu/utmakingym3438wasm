# YM3438 Browser Test (fixed build workflow)
The workflow uses a robust WASI SDK discovery step instead of assuming the extracted directory name.
After the Action succeeds, download the artifact and serve index.html over HTTP(S), not file://.
