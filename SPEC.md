# SPEC

## Conventions
- All times in seconds
- All times ET

## Structs
- `dist_params`: ZeroMeanSkewStudentT dist params

## Parameters
- `durations`: 5m, 15m, 1h, 4h, 1d
- `symbols`:
    - `{binance: BTCUSDT, pm_short: btc, pm_long: bitcoin}`
    - `{binance: ETHUSDT, pm_short: eth, pm_long: ethereum}`
    - `{binance: SOLUSDT, pm_short: sol, pm_long: solana}`
    - `{binance: XRPUSDT, pm_short: xrp, pm_long: xrp}`
- `history_start`: `2025-01-01T00:00:00Z`
- `base_timestep`: 1s
- `extra_timesteps`: [32s, 1024s]
- `features_per_symbol`:
    - `returns`: normalized log returns
    - `volume`: normalized log volume
- `features_per_timestep`:
    - `tow`: time-of-week encoded as `sin` + `cos`
- `features_horizon`:
    - `log`
    - `sqrt`
- `normalization`: z-score (fit on train only)
- `seq_length`
- `encoder_hidden_size`
- `encoder_num_layers`
- `encoder_dropout`
- `head_hidden_size`
- `head_num_layers`
- `head_dropout`
- `inference_cadence`: 1s
- `validation_split`
- `density`
- `optimizer`: AdamW
- `learning_rate`
- `weight_decay`
- `batch_size`
- `epochs`
- `kelly_fraction`
- `gain_threshold`
- `size_threshold`
- `per_market_bankroll`
- `dashboard_update`

## Helpers
- calc_prob(dist, price, ref_price)
    - returns cumulative probability of dist for x=-returns_since_ref (log)
- build_slug(coin, duration, start)
    - patterns:
        - 5m: `{pm_short}-updown-5m-{start_ts}`
        - 15m: `{pm_short}-updown-15m-{start_ts}`
        - 4h: `{pm_short}-updown-4h-{start_ts}`
        - 1h: `{pm_long}-up-or-down-{month}-{day}-{hour12}{am_pm}-et`
        - 1d: `{pm_long}-up-or-down-on-{month}-{day}`
    - there is a market start every time: now mod duration = 0

## Module 1: Crypto Data
- crypto_data_sync()
    - Sync binance historical trade data for `[history_start, now)`.
    - First monthly csv, then daily csv, then rest api for remaining.
    - No gaps.
- crypto_data_update(trade)
    - Called live on each trade to keep system updated.
- crypto_data_feature_sequence(end,horizon)
    - Returns deterministic feature sequence for all coins, for base step and extra steps. Horizon features static sequence also.

## Module 2: Crypto Live Data
- crypto_live_start()
    - Connects to Binance websockets for all configured symbols.
    - Calls crypto_data_update(trade) on each trade.
    - Calls manager_update_price(coin, price) on each book midprice change.

## Module 3: Model
- Characteristics:
    - Input: crypto_data_feature_sequence
    - Output: `dist_params` for all coins.
    - Encoder: LSTM.
    - Loss: NLL on returns after horizon.
- model_load(path)
    - Loads model artifact into memory.
- model_predict()
    - Returns `dist_params` for requested horizons.
- model_save(path)
    - Persists current model artifact.

## Module 4: Model Trainer
- model_trainer_run(train_start, train_end)
    - Builds training dataset from crypto_data_feature_sequence(end).
    - Apply validation split and only use density % of input datapoints.
    - Duration random, horizon random in 1..duration for any training input.
    - Trains model parameters.
    - Saves trained model with model_save(path).
- model_trainer_eval(eval_start, eval_end)
    - Computes evaluation metrics for trained model.

## Module 5: Model Runner
- model_runner_start()
    - Starts 1s inference loop.
    - On each tick: calls crypto_data_feature_sequence(end).
    - Calls model_predict(feature_sequence, horizons).
    - Calls manager_update_dists() with inferred params.
- model_runner_tick(end)
    - Single inference step version of the same loop.

## Module 6: PM Engine
- One instance per market.
- pm_engine_start(id)
    - Instance start.
    - Connects to market websockets and maintains updated orderbook and market-local state.
- pm_calc_prob()
    - If start <= now <= end: prob = calc_prob(dist)
    - If now < start: prob = calc_prob(dist_extra) - calc_prob(dist)
    - if now > end: prob already defined
- pm_engine_eval_action(shared_state)
    - Calculate fractional Kelly gain and sizing
    - Execute action if thresholds pass
    - Send multiorder (only if paper=False)
- pm_engine_report()
    - Reports market relevant data for dashboard:
    - link, coin, duration, bets_open, in_interval, end_time, ref_price, price, probability
    - best_bid_yes, best_ask_yes, position_net, pos_yes, pos_no, offer_yes, offer_no
    - net_profit, taker_fee_pct, maker_fee_pct, fee_exponent, reward_pct

## Module 7: Manager
- Owns `{price, ref_price, dist, extra_dist}` for every coin * duration.
- manager_start()
    - Finds current relevant markets and continually updates.
    - Uses `build_slug(coin,duration,end)` for discovery keys.
    - Starts and manages pm_engine instances for every market.
- manager_update_price(coin, price)
    - Updates shared price/ref_price state.
    - Calls manager_eval_actions(trigger).
- manager_update_dists()
    - Updates shared dist/extra_dist state for all coins and durations.
    - Calls manager_eval_actions(trigger).
- manager_eval_actions(trigger)
    - Runs `pm_engine_eval_action(...)` for managed market engines.
- manager_markets_report()
    - Returns report for all managed markets.
- manager_dist_report()
    - Report for variables

## Module 8: Dashboard
- Read-only module.
- Supports filters: coin, duration, bets_open, in_interval. All checkboxes lists.
- Shows table `manager_markets_report()`.
- Shows table `manager_dist_report()`.
- dashboard_run(filters)
    - Starts/keeps dashboard service running.