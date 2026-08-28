#!/bin/bash

pushd target/debug
./context_translate \
	--src-lang "Spanish" \
	--dst-lang "English" \
	--model Qwen3.5-35B-A3B-UD-Q4_K_L.gguf \
	--system-prompt /home/matias/Projects/context_translate/examples/manga/system_prompt.txt \
	--endpoint http://127.0.0.1:8081/v1/chat/completions \
	--src-csv ../../test_input.csv \
	--dst-csv ../../test_output.csv \
	--timeout-secs 120 \
	--llm-options /home/matias/Projects/context_translate/examples/ods/options.json \
	--api-key AAAAAA \
	--max-passes 3 \
	--pre-ctx 3 \
	--batch-size 20 \
	--pos-ctx 3 \
	--debug
popd
