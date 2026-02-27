# SPEC

## Structs
- `dist_params`: ZeroMeanSkewStudentT dist params

## Parameters
- `durations`: 5m, 15m, 1h, 4h, 1d
- `symbols`: `["BTCUSDT","ETHUSDT","SOLUSDT","XRPUSDT"]`
- `history_start`: `2025-01-01T00:00:00Z`
- `base_timestep`: 1s
- `extra_timesteps`: [32s, 1024s]
- `features_per_symbol`:
    - `returns`: normalized log returns
    - `volume`: normalized log volume
- `features_per_timestep`:
    - `tow`: time-of-week encoded as `sin` + `cos`
- `seq_length`

## Module 0: Nexus
{price, ref_price, dist, extra_dist} for every coin * duration
- nexus_update_price(coin, price)
    - Updates internal price for the coin
    - Calls pm_manager_eval_actions(trigger)
- nexus_update_dists()
    - Updates dists parameters for all coins, all durations
    - Calls pm_manager_eval_actions(trigger)

## Module 1: Crypto Data
- crypto_data_sync()
    - Sync binance historical trade data for `[history_start, now)`.
    - First monthly csv, then daily csv, then rest api for remaining
    - No gaps
- crypto_data_update(trade)
    - Called live on each trade to keep system updated
- crypto_data_feature_sequence(end)
    - Returns feature sequence for all coins, for base step and extra steps.
    - Deterministic

## Module 2: Crypto Live Data
- crypto_live_start()
    - Connects to Binance websockets for all configured symbols
    - Calls crypto_data_update(trade) on each trade
    - Calls nexus_update_price(coin, price) on each book midprice change

## Module 3: Model
- model_load(path)
    - Loads model artifact into memory
- model_predict(feature_sequence, horizons)
    - Returns `dist_params` for requested horizons
- model_save(path)
    - Persists current model artifact

## Module 4: Model Trainer
- model_trainer_run(train_start, train_end)
    - Builds training dataset from crypto_data_feature_sequence(end)
    - Trains model parameters
    - Saves trained model with model_save(path)
- model_trainer_eval(eval_start, eval_end)
    - Computes evaluation metrics for trained model

## Module 5: Model Runner
- model_runner_start()
    - Starts 1s inference loop
    - On each tick: calls crypto_data_feature_sequence(end)
    - Calls model_predict(feature_sequence, horizons)
    - Calls nexus_update_dists() with inferred params
- model_runner_tick(end)
    - Single inference step version of the same loop

## Module 6: PM Engine
- pm_engine_start(id)
    - Instance start
    - Reads relevant parameters
    - Connects to market websockets and starts maintaining an updated orderbook
    - eval_action when book changes
- pm_engine_eval_action()
    - Analizes potential action courses
    - Executes decided actions if paper = False
- pm_engine_report()
    - Reports on all market relevant data
    - [All dashboard fields here]

## Module 7: PM Manager
- pm_manager_start()
    - Finds current relevant markets, and continually updates
    - Fires and manages pm_engine instances for every market
- pm_manager_eval_actions(coin)
    - Runs `pm_engine_eval_action(...)` for managed market engines
- pm_manager_report()
    - Returns report for all managed markets

## Module 8: Dashboard
- dashboard_start()
    - Starts read-only dashboard service
- dashboard_snapshot()
    - Builds table snapshot from nexus + PM manager state
- dashboard_render()
    - Renders single-page table view
