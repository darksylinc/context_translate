context_translate is a versatile, open-source command-line interface (CLI) tool designed to streamline the translation process for dialogue, subtitles, and other text-based content. By leveraging advanced AI models, context_translate provides context-aware localizations, ensuring accurate and natural-sounding translations, especially for languages with unique structures like Japanese.

> [!WARNING]
> context_translate is in early alpha stage. We are not providing binaries out of the box yet.

# What does it do?

context_translate takes a CSV a table structured like this as input:

```csv
datablock_name;Collection;Text Contents
Unique Key 001;John;Hi! Did you enjoy the movie yesterday?
Unique Key 002;Anna;"Yes! I loved it!
But I think Cecilia fell asleep"
Unique Key 003;Cecilia;No, I did not!
```

| datablock_name | Collection | Text Contents                                    |
|----------------|------------|--------------------------------------------------|
| Unique Key 001 | John       | Hi! Did you enjoy the movie yesterday?           |
| Unique Key 002 | Anna       | Yes! I loved it!<br>But I think Cecilia fell asleep |
| Unique Key 003 | Cecilia    | No, I did not!                                   |

And outputs the following:

| datablock_name | Collection | Text Contents                                     | Original                                         | Original Back                                     | Remarks                                                                                                                                                                           |
|----------------|------------|---------------------------------------------------|--------------------------------------------------|---------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Unique Key 001 | John       | ¡Hola! ¿Disfrutaste la película ayer?             | Hi! Did you enjoy the movie yesterday?           | Hi! Did you enjoy the movie yesterday?            |                                                                                                                                                                                   |
| Unique Key 002 | Anna       | ¡Sí! ¡Me encantó!<br>Pero creo que Cecilia se durmió | Yes! I loved it!<br>But I think Cecilia fell asleep | Yes! I loved it!<br>But I think Cecilia fell asleep. | The use of "But" in the second line is a bit tricky to translate because it can be interpreted in different ways. I chose to use "pero" to maintain the flow of the conversation. |
| Unique Key 003 | Cecilia    | ¡No, no me dormí!                                 | No, I did not!                                   | No, I didn't fall asleep!                         |                                                                                                                                                                                   |

> [!NOTE]  
> Notice the original sentences contained 2 lines:
>
> ```
> Yes! I loved it!
> But I think Cecilia fell asleep.
> ```
>
> And this newline was respected in the translation:
>
> ```
> ¡Sí! ¡Me encantó!
> Pero creo que Cecilia se durmió
> ```
>
> But it is **not guaranteed** the AI will always respect it.

The "Original Back" is useful for translating into languages you don't understand to double-check the translation is accurate. The "Remarks" may contain additional information useful for making decisions.

1. The "datablock" column is optional, and contains a unique Key string useful for identifying lines when importing/exporting from other formats. This data is NOT sent to the AI.
2. The "Collection" column contains the speaker's name. This data is sent to the AI so the AI can track who is saying what. It does not necessarily have to be a speaker. For example it can be "Anna's thoughts" or "Anna's speech bubble", or "Onomatopoeia".


# Why?

It all started with YouTube auto-translating the title of an [Argentinean video](https://www.youtube.com/watch?v=0qDA1OsSFdA) "Mundial de facturas: ¿cuál es la más rica?" as "World of invoices: which is the richest?", to which my AI translation attempts also gave the same translation.

**However when an AI is given a transcript of the first 30 seconds of the video**, the AI correctly translates it as "World Cup of Pastries: Which is the Richest?" which is a perfect translation.

Turns out this tool is excellent for Japanese, as it is a highly contextual language.

# How to use

This tool was mostly tested against llama.cpp running Mistral 3.1 24B which is excellent for translations:

This is how I launch llama.cpp on an AMD Radeon 6800 XT:

```bash
llama-server -m mistralai_Mistral-Small-3.1-24B-Instruct-2503-Q4_K_M.gguf -ngl 99 --ctx-size 10144 --jinja --temp 0.95 --port 8081 --api-key API_KEY -fa --swa-full --cache-reuse 64 -ub 2048 -b 2048 -np 1 --mlock --log-colors --no-webui --metrics -ctk q4_0 -ctv q4_0 -dev Vulkan0
```

And then run this tool:

```bash
./context_translate \
	--src-lang English --dst-lang Spanish \
	--api-key API_KEY -m mistralai_Mistral-Small-3.1-24B-Instruct-2503-Q4_K_M.gguf \
	--src-csv "input.csv" \
	--dst-csv "output.csv" \
	--system-prompt examples/manga/system_prompt.txt \
	--llm-options examples/manga/options.json \
	--endpoint http://127.0.0.1:8081/v1/chat/completions \
	--timeout-secs 30 \
	--pre-ctx 2 \
	--batch-size 10 \
	--pos-ctx 2
```

The most important parameters are the 3 last ones and the timeout:

1. `--pre-ctx <n>` how many lines *previous* lines to give as context, per batch.
2. `--batch-size <n>` how many lines to translate per batch.
3. `--pos-ctx <n>` how many lines *subsequent* lines to give as context, per batch.
4. `--timeout <seconds>` If the AI takes longer than that, it aborts and retries. This is useful for iterations in which the AI starts hallucinating and going off the rails. Thus this puts a hard-stop. Note that larger batch-size and context values means the AI will take longer thus the timeout may have to be raised.

For example if using:
```
	--pre-ctx 1 \
	--batch-size 2 \
	--pos-ctx 1
```

the input lines are:

| Speaker | Text                         |
|---------|------------------------------|
| John    | Hi!                          |
| Anna    | Hi! How are you?             |
| John    | Fine, how are you?           |
| Anna    | So so, I’ve had better days. |
| John    | Oh really? What's wrong?     |

Then the AI will first see:

| Speaker | Text               |                  |
|---------|--------------------|------------------|
| John    | Hi!                | Being translated |
| Anna    | Hi! How are you?   | Being translated |
| John    | Fine, how are you? | As pos context   |

to translate the first two lines. Then it will see:

| Speaker | Text                         |                  |
|---------|------------------------------|------------------|
| Anna    | Hi! How are you?             | As pre context   |
| John    | Fine, how are you?           | Being translated |
| Anna    | So so, I’ve had better days. | Being translated |
| John    | Oh really? What's wrong?     | As pos context   |

And then finally:

| Speaker | Text                         |                  |
|---------|------------------------------|------------------|
| John    | Fine, how are you?           | As pre context   |
| Anna    | So so, I’ve had better days. | Being translated |
| John    | Oh really? What's wrong?     | Being translated |

There are no ideal settings. You may also find that multiple runs with different settings produce different results:

| pre-ctx | batch-size | pos-ctx | timeout |
|---------|------------|---------|---------|
| 2       | 6          | 2       | 30      |
| 6       | 6          | 6       | 60      |
| 4       | 2          | 4       | 30      |
| 1       | 2          | 1       | 25      |

Then pick the best-translated lines.

## Does it work with ChatGPT?

I don't know. But we use the OpenAI API endpoints so in theory it should work.

Just point `--endpoint https://api.openai.com/v1/chat/completions` and set the proper API KEY. We are not responsible if hit rate limits or burns your credits.

# Customizing the System Prompt

The [system_prompt.txt](examples/manga/system_prompt.txt) we include as example can be customized.

You can provide further instructions on how to interpret certain lines, how to translate certain words; or talk about the overall personality of certain characters for more accurate translations.

# Performance

**This tool is slow**. At the moment batches are not concurrent due to issues I've encountered with llama.cpp

Additionally, there may be opportunities to improve performance by changing how we send some context, which could improve prompt cache reuse.

# Blender Plugin

This project comes with a Blender plugin written against Blender 4.2 LTS.

After installing the plugin, go to `File` -> `Export` -> `Export Text Objects to CSV`.

All visible text objects will be exported to CSV. The Collection the text object belongs to will be used as the Speaker Name.

Once translated, use `File` -> `Import` -> `Import Text Objects to CSV` to duplicate each text object with its translated counterpart. They will be added together to a single new Collection.

# License

Under GNU GENERAL PUBLIC LICENSE (GPL) 3.0. See [LICENSE](./LICENSE). 