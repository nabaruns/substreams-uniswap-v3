create index position_snapshot_id_block_range_excl on "sgdXXX"."position_snapshot" using gist (id, block_range);
create index brin_position_snapshot on "sgdXXX"."position_snapshot" using brin(lower(block_range), coalesce(upper(block_range), 2147483647), vid);
create index position_snapshot_block_range_closed on "sgdXXX"."position_snapshot"(coalesce(upper(block_range), 2147483647)) where coalesce(upper(block_range), 2147483647) < 2147483647;
create index attr_6_0_position_snapshot_id on "sgdXXX"."position_snapshot" using btree("id");
create index attr_6_1_position_snapshot_owner on "sgdXXX"."position_snapshot" using btree(substring("owner", 1, 64));
create index attr_6_2_position_snapshot_pool on "sgdXXX"."position_snapshot" using gist("pool", block_range);
create index attr_6_3_position_snapshot_position on "sgdXXX"."position_snapshot" using gist("position", block_range);
create index attr_6_4_position_snapshot_block_number on "sgdXXX"."position_snapshot" using btree("block_number");
create index attr_6_5_position_snapshot_timestamp on "sgdXXX"."position_snapshot" using btree("timestamp");
create index attr_6_6_position_snapshot_liquidity on "sgdXXX"."position_snapshot" using btree("liquidity");
create index attr_6_7_position_snapshot_deposited_token_0 on "sgdXXX"."position_snapshot" using btree("deposited_token_0");
create index attr_6_8_position_snapshot_deposited_token_1 on "sgdXXX"."position_snapshot" using btree("deposited_token_1");
create index attr_6_9_position_snapshot_withdrawn_token_0 on "sgdXXX"."position_snapshot" using btree("withdrawn_token_0");
create index attr_6_10_position_snapshot_withdrawn_token_1 on "sgdXXX"."position_snapshot" using btree("withdrawn_token_1");
create index attr_6_11_position_snapshot_collected_fees_token_0 on "sgdXXX"."position_snapshot" using btree("collected_fees_token_0");
create index attr_6_12_position_snapshot_collected_fees_token_1 on "sgdXXX"."position_snapshot" using btree("collected_fees_token_1");
create index attr_6_13_position_snapshot_transaction on "sgdXXX"."position_snapshot" using gist("transaction", block_range);
create index attr_6_14_position_snapshot_fee_growth_inside_0_last_x128 on "sgdXXX"."position_snapshot" using btree("fee_growth_inside_0_last_x128");
create index attr_6_15_position_snapshot_fee_growth_inside_1_last_x128 on "sgdXXX"."position_snapshot" using btree("fee_growth_inside_1_last_x128");