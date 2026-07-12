/* judgemark-v4.js */

const leaderboardDataJudgemarkV4 = `
model,score,ci_low,ci_high,cost,chart
claude-opus-4-6,0.907256,0.867410,0.967750,$39.37,results/judgemark-v4/charts/multichart_claude-opus-4-6_seed_temp0_prompt_reasoning_trial_01__claude-opus-4-6.png
gpt-5.5,0.878134,0.853332,0.924747,$30.44,results/judgemark-v4/charts/multichart_openai__gpt-5_5_seed_temp0_prompt_reasoning_trial_01__openai_gpt-5_5.png
claude-opus-4-7,0.839612,0.804731,0.894923,$48.75,results/judgemark-v4/charts/multichart_claude-opus-4-7_seed_temp0_prompt_reasoning_trial_01__claude-opus-4-7.png
claude-sonnet-4-6,0.821468,0.782343,0.882974,$23.36,results/judgemark-v4/charts/multichart_claude-sonnet-4-6_seed_temp0_prompt_reasoning_trial_01__claude-sonnet-4-6.png
gemini-3.1-pro-preview,0.786853,0.738905,0.851353,$23.07,results/judgemark-v4/charts/multichart_google__gemini-3_1-pro-preview_seed_temp0_prompt_reasoning_trial_01__google_gemini-3_1-pro-preview.png
claude-opus-4-8,0.779576,0.743971,0.838340,$50.42,results/judgemark-v4/charts/multichart_claude-opus-4-8_seed_temp0_prompt_reasoning_trial_01__claude-opus-4-8.png
zai-org/GLM-5.2,0.731905,0.679917,0.805355,$8.28,results/judgemark-v4/charts/multichart_z-ai__glm-5_2_seed_temp0_prompt_reasoning_trial_01__z-ai_glm-5_2.png
google/gemma-4-31B-it,0.723051,0.680818,0.793728,$0.82,results/judgemark-v4/charts/multichart_google__gemma-4-31b-it_seed_temp0_prompt_reasoning_trial_01__google_gemma-4-31b-it.png
gpt-5.4,0.720762,0.684801,0.781918,$15.24,results/judgemark-v4/charts/multichart_gpt-5_4_seed_temp0_prompt_reasoning_trial_01__openai_gpt-5_4.png
claude-sonnet-5,0.708864,0.670097,0.777549,$29.19,results/judgemark-v4/charts/multichart_claude-sonnet-5_seed_temp0_prompt_reasoning_trial_01__claude-sonnet-5.png
zai-org/GLM-5.1,0.672081,0.623536,0.740827,$8.06,results/judgemark-v4/charts/multichart_z-ai__glm-5_1_seed_temp0_prompt_reasoning_trial_01__z-ai_glm-5_1.png
Qwen/Qwen3.5-27B,0.605331,0.567596,0.680937,$1.76,results/judgemark-v4/charts/multichart_Qwen__Qwen3_5-27B_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_5-27b.png
gemini-3.1-flash-lite-preview,0.588214,0.550091,0.658023,$1.54,results/judgemark-v4/charts/multichart_gemini-3_1-flash-lite-preview_seed_temp0_prompt_reasoning_trial_01__google_gemini-3_1-flash-lite-preview.png
XiaomiMiMo/MiMo-V2.5-Pro,0.579018,0.539689,0.650709,$5.79,results/judgemark-v4/charts/multichart_xiaomi__mimo-v2_5-pro_seed_temp0_prompt_reasoning_trial_01__xiaomi_mimo-v2_5-pro.png
moonshotai/Kimi-K2.6,0.572626,0.547558,0.633247,$6.51,results/judgemark-v4/charts/multichart_kimi-k2_6_seed_temp0_prompt_reasoning_trial_01__kimi-k2_6.png
Qwen/Qwen3.5-397B-A17B,0.542445,0.485310,0.620456,$3.20,results/judgemark-v4/charts/multichart_Qwen__Qwen3_5-397B-A17B_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_5-397b-a17b.png
Qwen/Qwen3.6-27B,0.533686,0.512794,0.593391,$3.56,results/judgemark-v4/charts/multichart_qwen__qwen3_6-27b_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_6-27b.png
google/gemma-4-26B-A4B-it,0.530234,0.484937,0.608299,$0.61,results/judgemark-v4/charts/multichart_google__gemma-4-26b-a4b-it_seed_temp0_prompt_reasoning_trial_01__google_gemma-4-26b-a4b-it.png
nvidia/NVIDIA-Nemotron-3-Ultra-550B-A55B-NVFP4,0.529064,0.499723,0.592820,$3.30,results/judgemark-v4/charts/multichart_nvidia__nemotron-3-ultra-550b-a55b_seed_temp0_prompt_reasoning_trial_01__nvidia_nemotron-3-ultra-550b-a55b.png
grok-4.3,0.495693,0.457153,0.573230,$9.71,results/judgemark-v4/charts/multichart_x-ai__grok-4_3_seed_temp0_prompt_reasoning_trial_01__x-ai_grok-4_3.png
deepseek-ai/DeepSeek-V4-Pro,0.471182,0.416774,0.563053,$2.94,results/judgemark-v4/charts/multichart_deepseek__deepseek-v4-pro_seed_temp0_prompt_reasoning_trial_01__deepseek_deepseek-v4-pro.png
gemini-3-flash-preview,0.461171,0.424996,0.536712,$3.14,results/judgemark-v4/charts/multichart_gemini-3-flash-preview_seed_temp0_prompt_reasoning_trial_01__google_gemini-3-flash-preview.png
qwen3.6-max-preview,0.450786,0.403048,0.542877,$7.36,results/judgemark-v4/charts/multichart_qwen__qwen3_6-max-preview_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_6-max-preview.png
Qwen/Qwen3.5-35B-A3B,0.405004,0.370493,0.477875,$1.38,results/judgemark-v4/charts/multichart_Qwen__Qwen3_5-35B-A3B_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_5-35b-a3b.png
deepseek-ai/DeepSeek-V4-Flash,0.367862,0.340758,0.450511,$0.78,results/judgemark-v4/charts/multichart_deepseek__deepseek-v4-flash_seed_temp0_prompt_reasoning_trial_01__deepseek_deepseek-v4-flash.png
Qwen/Qwen3.6-35B-A3B,0.326547,0.305251,0.403566,$1.89,results/judgemark-v4/charts/multichart_qwen__qwen3_6-35b-a3b_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_6-35b-a3b.png
Qwen/Qwen3.5-9B,0.324392,0.288405,0.415527,$0.56,results/judgemark-v4/charts/multichart_Qwen__Qwen3_5-9B_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_5-9b.png
qwen3.6-flash,0.311761,0.276205,0.400733,$1.74,results/judgemark-v4/charts/multichart_qwen__qwen3_6-flash_seed_temp0_prompt_reasoning_trial_01__qwen_qwen3_6-flash.png
stepfun-ai/Step-3.7-Flash,0.280475,0.262261,0.365120,$3.03,results/judgemark-v4/charts/multichart_stepfun__step-3_7-flash_seed_temp0_prompt_reasoning_trial_01__stepfun_step-3_7-flash.png
gpt-5.4-nano,0.275388,0.247464,0.363286,$1.25,results/judgemark-v4/charts/multichart_gpt-5_4-nano_seed_temp0_prompt_reasoning_trial_01__openai_gpt-5_4-nano.png
gpt-5.4-mini,0.273221,0.248656,0.357704,$4.56,results/judgemark-v4/charts/multichart_gpt-5_4-mini_seed_temp0_prompt_reasoning_trial_01__openai_gpt-5_4-mini.png
mistralai/Mistral-Small-4-119B-2603,0.164969,0.150587,0.255205,$0.92,results/judgemark-v4/charts/multichart_mistralai__Mistral-Small-4-119B-2603_seed_temp0_prompt_reasoning_trial_01__mistralai_mistral-small-2603.png
gpt-oss-120b,0.154748,0.145411,0.232810,$0.62,results/judgemark-v4/charts/multichart_openai__gpt-oss-120b_seed_temp0_prompt_reasoning_trial_01__openai_gpt-oss-120b.png
`
