extern crate core;

mod pb;
mod abi;
mod utils;
mod rpc;
mod eth;

use std::collections::HashMap;
use std::ops::Neg;
use std::str::FromStr;
use bigdecimal::{BigDecimal, FromPrimitive};
use num_bigint::BigInt;
use substreams::errors::Error;
use substreams::{Hex, log, proto, store};
use substreams::store::{StoreAddBigFloat, StoreAppend, StoreGet, StoreSet};
use substreams_ethereum::pb::eth as ethpb;
use bigdecimal::ToPrimitive;
use crate::pb::uniswap::{Burn, Event, Flash, Mint, Pool, PoolInitialization, SqrtPriceUpdate, SqrtPriceUpdates, Tick, UniswapToken, UniswapTokens};
use crate::pb::uniswap::event::Type;
use crate::pb::uniswap::event::Type::Swap as SwapEvent;
use crate::pb::uniswap::event::Type::Burn as BurnEvent;
use crate::pb::uniswap::event::Type::Mint as MintEvent;
use crate::utils::{get_last_pool_tick, big_decimal_exponated, safe_div};

#[substreams::handlers::map]
pub fn map_pools_created(block: ethpb::v1::Block) -> Result<pb::uniswap::Pools, Error> {
    let mut output = pb::uniswap::Pools { pools: vec![] };
    for trx in block.transaction_traces {
        for call in trx.calls.iter() {
            if hex::encode(&call.address) != utils::UNISWAP_V3_FACTORY {
                continue;
            }

            for call_log in call.logs.iter() {
                if !abi::factory::events::PoolCreated::match_log(&call_log) {
                    continue
                }

                let event = abi::factory::events::PoolCreated::must_decode(&call_log);
                output.pools.push(Pool {
                    address: Hex(&call_log.data[44..64]).to_string(),
                    token0_address: Hex(&event.token0).to_string(),
                    token1_address: Hex(&event.token1).to_string(),
                    creation_transaction_id: Hex(&trx.hash).to_string(),
                    fee: event.fee.as_u32(),
                    block_num: block.number.to_string(),
                    log_ordinal: call_log.ordinal,
                    tick_spacing: event.tick_spacing.to_i32().unwrap(),
                });
            }
        }
    }

    Ok(output)
}

#[substreams::handlers::map]
pub fn map_uniswap_tokens(pools: pb::uniswap::Pools) -> Result<UniswapTokens, Error> {
    let mut output = UniswapTokens { uniswap_tokens: vec![] };

    //todo(colin): do we really care enough to cache tokens?
    let mut cached_tokens = HashMap::new();

    for pool in pools.pools {
        log::info!("pool address: {}, trx_hash: {}", pool.address, pool.creation_transaction_id);

        // TODO: refactor this...
        let mut can_add_pool = true;
        let mut uniswap_token0_option: Option<UniswapToken> = None;
        let mut uniswap_token1_option: Option<UniswapToken> = None;
        let mut uniswap_token0 = UniswapToken {
            address: "".to_string(),
            name: "".to_string(),
            symbol: "".to_string(),
            decimals: 0,
            whitelist_pools: vec![]
        };
        let mut uniswap_token1 = UniswapToken {
            address: "".to_string(),
            name: "".to_string(),
            symbol: "".to_string(),
            decimals: 0,
            whitelist_pools: vec![]
        };

        let token0_address: String = pool.token0_address;
        if !cached_tokens.contains_key(&token0_address) {
            uniswap_token0_option = rpc::create_uniswap_token(&token0_address);
            if uniswap_token0_option.is_none() {
                can_add_pool = false;
            } else {
                uniswap_token0 = uniswap_token0_option.unwrap();
                cached_tokens.insert(String::from(&token0_address), true);

                if !output.uniswap_tokens.contains(&uniswap_token0) {
                    if utils::WHITELIST_TOKENS.contains(&token0_address.as_str()) {
                        uniswap_token0.whitelist_pools.push(String::from(&pool.address))
                    }
                }
            }
        }

        let token1_address: String = pool.token1_address;
        if !cached_tokens.contains_key(&token1_address) {
            uniswap_token1_option = rpc::create_uniswap_token(&token1_address);
            if uniswap_token1_option.is_none() {
                can_add_pool = false;
            } else {
                uniswap_token1 = uniswap_token1_option.unwrap();
                cached_tokens.insert(String::from(&token1_address), true);

                if !output.uniswap_tokens.contains(&uniswap_token1) {
                    if utils::WHITELIST_TOKENS.contains(&token1_address.as_str()) {
                        uniswap_token1.whitelist_pools.push(String::from(&pool.address))
                    }
                }
            }
        }

        if can_add_pool {
            output.uniswap_tokens.push(uniswap_token0);
            output.uniswap_tokens.push(uniswap_token1);
        }

    }

    Ok(output)
}

#[substreams::handlers::store]
pub fn store_uniswap_tokens(uniswap_tokens: UniswapTokens, output_set: StoreSet) {
    for uniswap_token in uniswap_tokens.uniswap_tokens {
        output_set.set(
            1,
            format!("token:{}", uniswap_token.address),
            &proto::encode(&uniswap_token).unwrap()
        );
    }
}

#[substreams::handlers::store]
pub fn store_uniswap_tokens_whitelist_pools(uniswap_tokens: UniswapTokens, output_append: StoreAppend) {
    for uniswap_token in uniswap_tokens.uniswap_tokens {
        for pools in uniswap_token.whitelist_pools {
            output_append.append(
                1,
                format!("token:{}", uniswap_token.address),
                &format!("{};", pools.to_string())
            )
        }
    }
}

#[substreams::handlers::map]
pub fn map_pools_initialized(block: ethpb::v1::Block) -> Result<pb::uniswap::PoolInitializations, Error> {
    let mut output = pb::uniswap::PoolInitializations { pool_initializations: vec![] };
    for trx in block.transaction_traces {
        for log in trx.receipt.unwrap().logs {
            if !abi::pool::events::Initialize::match_log(&log) {
                continue;
            }

            let event = abi::pool::events::Initialize::must_decode(&log);
            output.pool_initializations.push(PoolInitialization{
                address: Hex(&log.address).to_string(),
                initialization_transaction_id: Hex(&trx.hash).to_string(),
                log_ordinal: log.ordinal,
                tick: event.tick.to_string(),
                sqrt_price: event.sqrt_price_x96.to_string(),
            });
        }
    }

    Ok(output)
}

// map will take the block and the init mapper for the pools
// mapper -> sqrt price de init et swap
#[substreams::handlers::map]
pub fn map_sqrt_price(block: ethpb::v1::Block) -> Result<SqrtPriceUpdates, Error> {
    let mut output = SqrtPriceUpdates { sqrt_prices: vec![] };

    for trx in block.transaction_traces {
        for log in trx.receipt.unwrap().logs {
            if abi::pool::events::Initialize::match_log(&log) {
                let event = abi::pool::events::Initialize::must_decode(&log);
                output.sqrt_prices.push(SqrtPriceUpdate {
                    pool_address: Hex(&log.address).to_string(),
                    ordinal: log.ordinal,
                    sqrt_price: event.sqrt_price_x96.to_string(),
                    tick: event.tick.to_string(),
                })
            } else if abi::pool::events::Swap::match_log(&log){
                let event = abi::pool::events::Swap::must_decode(&log);
                output.sqrt_prices.push(SqrtPriceUpdate {
                    pool_address: Hex(&log.address).to_string(),
                    ordinal: log.ordinal,
                    sqrt_price: event.sqrt_price_x96.to_string(),
                    tick: event.tick.to_string(),
                })
            }
        }
    }

    Ok(output)
}

#[substreams::handlers::store]
pub fn store_sqrt_price(sqrt_prices: SqrtPriceUpdates, output: StoreSet) {
    for sqrt_price in sqrt_prices.sqrt_prices {
        output.set(
            1,
            format!("sqrt_price:{}", sqrt_price.pool_address),
            &proto::encode(&sqrt_price).unwrap()
        )
    }
}

#[substreams::handlers::store]
pub fn store_pools_initialization(pools: pb::uniswap::PoolInitializations, output_set: StoreSet) {
    for init in pools.pool_initializations {
        output_set.set(
            1,
            format!("pool_init:{}", init.address),
            &proto::encode(&init).unwrap()
        );
    }
}

// note: here we do something a bit different than the unisap-v3-subgraph,
// we have to ONLY set the pools which contain valid tokens and not just any pool OR do we want to
// store pools with invalid tokens? this is what the uniswap-v3-subgraph seems to be doing...
#[substreams::handlers::store]
pub fn store_pools(pools: pb::uniswap::Pools, tokens_store: StoreGet, output: StoreSet) {
    for pool in pools.pools {
        match tokens_store.get_last(&format!("token:{}", pool.token0_address)) {
            None => {
                continue
            }
            Some(_) => {
                match tokens_store.get_last(&format!("token:{}", pool.token1_address)) {
                    None => {
                        continue
                    }
                    Some(_) => {
                        output.set(
                            pool.log_ordinal,
                            format!("pool:{}", pool.address),
                            &proto::encode(&pool).unwrap(),
                        );
                    }
                }
            }
        }
    }
}

#[substreams::handlers::map]
pub fn map_burns_swaps_mints(block: ethpb::v1::Block, pools_store: StoreGet) -> Result<pb::uniswap::Events, Error> {
    let mut output = pb::uniswap::Events { events: vec![] };
    for trx in block.transaction_traces {
        for log in trx.receipt.unwrap().logs.iter() {
            let pool_key = &format!("pool:{}", Hex(&log.address).to_string());

            if abi::pool::events::Swap::match_log(log) {
                let swap = abi::pool::events::Swap::must_decode(log);
                match pools_store.get_last(pool_key) {
                    None => {
                        panic!("invalid swap. pool does not exist. pool address {} transaction {}", Hex(&log.address).to_string(), Hex(&trx.hash).to_string());
                    }
                    Some(pool_bytes) => {
                        // log::info!("trx id: {}", &Hex(trx.hash.clone()).to_string());
                        let pool: Pool = proto::decode(&pool_bytes).unwrap();

                        output.events.push(Event{
                            log_ordinal: log.ordinal,
                            pool_address: pool.address.to_string(),
                            token0: pool.token0_address.to_string(),
                            token1: pool.token1_address.to_string(),
                            fee: pool.fee.to_string(),
                            transaction_id: Hex(&trx.hash).to_string(),
                            timestamp: block.header.as_ref().unwrap().timestamp.as_ref().unwrap().seconds as u64,
                            r#type: Some(SwapEvent(pb::uniswap::Swap{
                                sender: Hex(&swap.sender).to_string(),
                                recipient: Hex(&swap.recipient).to_string(),
                                amount_0: swap.amount0.to_string(), // big_decimal?
                                amount_1: swap.amount1.to_string(), // big_decimal?
                                sqrt_price: swap.sqrt_price_x96.to_string(), // big_decimal?
                                liquidity: swap.liquidity.to_string(),
                                tick: swap.tick.to_i32().unwrap(),
                            })),
                        });
                    }
                }
            }

            if abi::pool::events::Burn::match_log(log) {
                let burn = abi::pool::events::Burn::must_decode(log);

                match pools_store.get_last(pool_key) {
                    None => {
                        panic!("invalid burn. pool does not exist. pool address {} transaction {}", Hex(&log.address).to_string(), Hex(&trx.hash).to_string());
                    }
                    Some(pool_bytes) => {
                        let pool: Pool = proto::decode(&pool_bytes).unwrap();

                        output.events.push(Event{
                            log_ordinal: log.ordinal,
                            pool_address: pool.address.to_string(),
                            token0: pool.token0_address.to_string(),
                            token1: pool.token1_address.to_string(),
                            fee: pool.fee.to_string(),
                            transaction_id: Hex(&trx.hash).to_string(),
                            timestamp: block.header.as_ref().unwrap().timestamp.as_ref().unwrap().seconds as u64,
                            r#type: Some(BurnEvent(Burn{
                                owner: Hex(&burn.owner).to_string(),
                                amount_0: burn.amount0.to_string(),
                                amount_1: burn.amount1.to_string(),
                                tick_lower: burn.tick_lower.to_i32().unwrap(),
                                tick_upper: burn.tick_upper.to_i32().unwrap(),
                                amount: burn.amount.to_string(),
                            })),
                        });
                    }
                }
            }

            if abi::pool::events::Mint::match_log(log) {
                let mint = abi::pool::events::Mint::must_decode(log);

                match pools_store.get_last(pool_key) {
                    None => {
                        panic!("invalid mint. pool does not exist. pool address {} transaction {}", Hex(&log.address).to_string(), Hex(&trx.hash).to_string());
                    }
                    Some(pool_bytes) => {
                        let pool: Pool = proto::decode(&pool_bytes).unwrap();

                        output.events.push(Event{
                            log_ordinal: log.ordinal,
                            pool_address: pool.address.to_string(),
                            token0: pool.token0_address.to_string(),
                            token1: pool.token1_address.to_string(),
                            fee: pool.fee.to_string(),
                            transaction_id: Hex(&trx.hash).to_string(),
                            timestamp: block.header.as_ref().unwrap().timestamp.as_ref().unwrap().seconds as u64,
                            r#type: Some(MintEvent(Mint{
                                owner: Hex(&mint.owner).to_string(),
                                sender: Hex(&mint.sender).to_string(),
                                amount_0: mint.amount0.to_string(), // big_decimal?
                                amount_1: mint.amount1.to_string(), // big_decimal?
                                tick_lower: mint.tick_lower.to_i32().unwrap(),
                                tick_upper: mint.tick_upper.to_i32().unwrap(),
                                amount: mint.amount.to_string(), // big_decimal?
                            })),
                        });
                    }
                }
            }
        }
    }

    Ok(output)
}

#[substreams::handlers::store]
pub fn store_swaps(events: pb::uniswap::Events, output_set: StoreSet) {
    for event in events.events {
        match event.r#type.unwrap() {
            Type::Swap(swap) => {
                output_set.set(
                    event.log_ordinal,
                    format!("pool:{}", event.pool_address),
                    &proto::encode(&swap).unwrap(),
                );
            }
            _ => {}
        }
    }
}

#[substreams::handlers::store]
pub fn store_ticks(events: pb::uniswap::Events, output_set: StoreSet) {
    for event in events.events {
        match event.r#type.unwrap() {
            Type::Swap(_) => {}
            Type::Burn(_) => {
                // todo
            }
            Type::Mint(mint) => {
                let tick_lower_big_int = BigInt::from_str(&mint.tick_lower.to_string()).unwrap();
                let tick_lower_price0=  big_decimal_exponated(BigDecimal::from_f64(1.0001).unwrap().with_prec(100), tick_lower_big_int);
                let tick_lower_price1 = safe_div(BigDecimal::from(1 as i32), &tick_lower_price0);

                let tick_lower: Tick = Tick{
                    pool_address: event.pool_address.to_string(),
                    idx: mint.tick_lower.to_string(),
                    price0: tick_lower_price0.to_string(),
                    price1: tick_lower_price1.to_string(),
                };

                output_set.set(
                    event.log_ordinal,
                    format!("tick:{}:pool:{}", mint.tick_lower.to_string(), event.pool_address.to_string()),
                    &proto::encode(&tick_lower).unwrap(),
                );

                let tick_upper_big_int = BigInt::from_str(&mint.tick_upper.to_string()).unwrap();
                let tick_upper_price0=  big_decimal_exponated(BigDecimal::from_f64(1.0001).unwrap().with_prec(100), tick_upper_big_int);
                let tick_upper_price1 = safe_div(BigDecimal::from(1 as i32), &tick_upper_price0);
                let tick_upper: Tick = Tick{
                    pool_address: event.pool_address.to_string(),
                    idx: mint.tick_upper.to_string(),
                    price0: tick_upper_price0.to_string(),
                    price1: tick_upper_price1.to_string(),
                };

                output_set.set(
                    event.log_ordinal,
                    format!("tick:{}:pool:{}", mint.tick_upper.to_string(), event.pool_address.to_string()),
                    &proto::encode(&tick_upper).unwrap(),
                );
            }
        }
    }
}

#[substreams::handlers::store]
pub fn store_liquidity(events: pb::uniswap::Events, swap_store: StoreGet, pool_init_store: StoreGet, output: StoreAddBigFloat) {
    for event in events.events {
        log::debug!("transaction id: {}", event.transaction_id);
        if event.r#type.is_some() {
            match event.r#type.unwrap() {
                Type::Swap(swap) => {
                    let amount0 = BigDecimal::from_str(swap.amount_0.as_str()).unwrap();
                    let amount1 = BigDecimal::from_str(swap.amount_1.as_str()).unwrap();
                    let liquidity = BigDecimal::from_str(swap.liquidity.as_str()).unwrap();
                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token0),
                        &amount0
                    );
                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token1),
                        &amount1
                    );

                    // todo(colin): this is incorrect
                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:liquidity", event.pool_address),
                        &liquidity
                    );
                }
                Type::Burn(burn) => {
                    //get pool info from last swap event
                    let amount = BigDecimal::from_str(burn.amount.as_str()).unwrap();
                    let amount0 = BigDecimal::from_str(burn.amount_0.as_str()).unwrap();
                    let amount1 = BigDecimal::from_str(burn.amount_1.as_str()).unwrap();
                    let tick_lower = BigDecimal::from_str(burn.tick_lower.to_string().as_str()).unwrap();
                    let tick_upper = BigDecimal::from_str(burn.tick_upper.to_string().as_str()).unwrap();
                    let tick = get_last_pool_tick(&pool_init_store, &swap_store, &event.pool_address).unwrap();

                    if tick_lower <= tick && tick <= tick_upper {
                        output.add(
                            event.log_ordinal,
                            format!("pool:{}:liquidity", event.pool_address),
                            &amount.neg()
                        );
                    }

                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token0),
                        &amount0.neg()
                    );
                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token1),
                        &amount1.neg()
                    );
                }
                Type::Mint(mint) => {
                    //get pool info from last swap event
                    let amount = BigDecimal::from_str(mint.amount.as_str()).unwrap();
                    let amount0 = BigDecimal::from_str(mint.amount_0.as_str()).unwrap();
                    let amount1 = BigDecimal::from_str(mint.amount_1.as_str()).unwrap();
                    let tick_lower = BigDecimal::from_str(mint.tick_lower.to_string().as_str()).unwrap();
                    let tick_upper = BigDecimal::from_str(mint.tick_upper.to_string().as_str()).unwrap();
                    let tick = get_last_pool_tick(&pool_init_store, &swap_store, &event.pool_address).unwrap();

                    if tick_lower <= tick && tick <= tick_upper {
                        output.add(
                            event.log_ordinal,
                            format!("pool:{}:liquidity", event.pool_address),
                            &amount
                        );
                    }

                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token0),
                        &amount0
                    );
                    output.add(
                        event.log_ordinal,
                        format!("pool:{}:token:{}:total_value_locked", event.pool_address, event.token1),
                        &amount1
                    );
                }
            }
        }
    }
}

//todo: alex wants this store split, have a store which computes the prices (maybe this store)
// which will simply calculate the prices AND have another store which calculates the
// `derivedEth`price of each token
#[substreams::handlers::store]
pub fn store_prices(
    sqrt_price_updates: SqrtPriceUpdates,
    pools_store: StoreGet,
    tokens_store: StoreGet,
    output: StoreSet
) {
    for sqrt_price_update in sqrt_price_updates.sqrt_prices {
        let pool = utils::get_last_pool(&pools_store, sqrt_price_update.pool_address.as_str()).unwrap();

        log::debug!("pool addr: {}, token0 addr: {}, token1 addr: {}", pool.address, pool.token0_address, pool.token1_address);
        let token_0 = utils::get_last_token(&tokens_store, pool.token0_address.as_str()).unwrap();
        let token_1 = utils::get_last_token(&tokens_store, pool.token1_address.as_str()).unwrap();

        log::info!("token 0 addr: {}, token 0 decimals: {}, tokens 0 symbol: {}, token 0 name: {}", token_0.address, token_0.decimals, token_0.symbol, token_0.name);
        log::info!("token 1 addr: {}, token 1 decimals: {}, tokens 1 symbol: {}, token 1 name: {}", token_1.address, token_1.decimals, token_1.symbol, token_1.name);

        let sqrt_price = BigDecimal::from_str(sqrt_price_update.sqrt_price.as_str()).unwrap();
        log::info!("sqrtPrice: {}", sqrt_price.to_string());

        let tokens_price: (BigDecimal, BigDecimal) = utils::sqrt_price_x96_to_token_prices(&sqrt_price, &token_0, &token_1);
        log::debug!("token prices: {} {}", tokens_price.0, tokens_price.1);

        output.set(
            sqrt_price_update.ordinal,
            format!("pool:{}:token:{}:price", pool.address, token_0.address),
            &Vec::from(tokens_price.0.to_string())
        );
        output.set(
            sqrt_price_update.ordinal,
            format!("pool:{}:token:{}:price", pool.address, token_1.address),
            &Vec::from(tokens_price.1.to_string())
        );
    }
}

#[substreams::handlers::store]
pub fn store_derived_eth_prices(
    sqrt_price_updates: SqrtPriceUpdates,
    liquidity_store: StoreGet,
    pools_store: StoreGet,
    pools_init_store: StoreGet,
    swap_store: StoreGet,
    tokens_store: StoreGet,
    whitelist_pools_store: StoreGet,
    output: StoreSet
) {
    for sqrt_price_update in sqrt_price_updates.sqrt_prices {
        log::debug!("fetching pool: {}", sqrt_price_update.pool_address);
        let pool = utils::get_last_pool(&pools_store, sqrt_price_update.pool_address.as_str()).unwrap();

        let token_0 = utils::get_last_token(&tokens_store, pool.token0_address.as_str()).unwrap();
        let token_1 = utils::get_last_token(&tokens_store, pool.token1_address.as_str()).unwrap();

        log::info!("token 0 addr: {}, token 0 decimals: {}, tokens 0 symbol: {}, token 0 name: {}", token_0.address, token_0.decimals, token_0.symbol, token_0.name);
        log::info!("token 1 addr: {}, token 1 decimals: {}, tokens 1 symbol: {}, token 1 name: {}", token_1.address, token_1.decimals, token_1.symbol, token_1.name);

        let token0_derived_eth_price = utils::find_eth_per_token(
            sqrt_price_update.ordinal,
            &token_0.address.as_str(),
            &pools_store,
            &pools_init_store,
            &swap_store,
            &tokens_store,
            &whitelist_pools_store,
            &liquidity_store,
        );
        log::info!("token0_derived_eth_price: {}", token0_derived_eth_price);

        let token1_derived_eth_price = utils::find_eth_per_token(
            sqrt_price_update.ordinal,
            &token_1.address.as_str(),
            &pools_store,
            &pools_init_store,
            &swap_store,
            &tokens_store,
            &whitelist_pools_store,
            &liquidity_store,
        );
        log::info!("token1_derived_eth_price: {}", token1_derived_eth_price);

        output.set(
            sqrt_price_update.ordinal,
            format!("token:{}:dprice:eth", token_0.address),
            &Vec::from(token0_derived_eth_price.to_string())
        );
        output.set(
            sqrt_price_update.ordinal,
            format!("token:{}:dprice:eth", token_1.address),
            &Vec::from(token1_derived_eth_price.to_string())
        );
    }
}

#[substreams::handlers::map]
pub fn map_fees(block: ethpb::v1::Block) -> Result<pb::uniswap::Fees, Error> {
    let mut out = pb::uniswap::Fees { fees: vec![] };

    for trx in block.transaction_traces {
        for log in trx.receipt.unwrap().logs {
            if !abi::factory::events::FeeAmountEnabled::match_log(&log) {
                continue;
            }

            let ev = abi::factory::events::FeeAmountEnabled::must_decode(&log);

            out.fees.push(pb::uniswap::Fee {
                fee: ev.fee.as_u32(),
                tick_spacing: ev.tick_spacing.to_i32().unwrap(),
            });
        }
    }

    Ok(out)
}

#[substreams::handlers::store]
pub fn store_fees(block: ethpb::v1::Block, output: store::StoreSet) {
    for trx in block.transaction_traces {
        for log in trx.receipt.unwrap().logs {
            if !abi::factory::events::FeeAmountEnabled::match_log(&log) {
                continue;
            }

            let event = abi::factory::events::FeeAmountEnabled::must_decode(&log);

            let fee = pb::uniswap::Fee {
                fee: event.fee.as_u32(),
                tick_spacing: event.tick_spacing.to_i32().unwrap()
            };

            output.set(
                log.ordinal,
                format!("fee:{}:{}", fee.fee, fee.tick_spacing),
                &proto::encode(&fee).unwrap(),
            );
        }
    }
}

#[substreams::handlers::map]
pub fn map_flashes(block: ethpb::v1::Block) -> Result<pb::uniswap::Flashes, Error> {
    let mut out = pb::uniswap::Flashes { flashes: vec![] };

    for trx in block.transaction_traces {
        for call in trx.calls.iter() {
            for call_log in call.logs.iter() {
                if !abi::pool::events::Flash::match_log(&call_log) {
                    continue;
                }

                let flash = abi::pool::events::Flash::must_decode(&call_log);
                log::debug!("{:?}", flash);

                out.flashes.push(Flash{
                    sender: Hex(&flash.sender).to_string(),
                    recipient: Hex(&flash.recipient).to_string(),
                    amount_0: "".to_string(),
                    amount_1: "".to_string(),
                    paid_0: "".to_string(),
                    paid_1: "".to_string(),
                    transaction_id: Hex(&trx.hash).to_string(),
                    log_ordinal: call_log.ordinal,
                });
            }
        }
    }

    Ok(out)
}