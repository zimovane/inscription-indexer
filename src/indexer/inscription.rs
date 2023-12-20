use super::{
    database::{dump_deploy_inscription, dump_mint_inscription, update_indexed_block},
    Indexer, Inscription, InscriptionFieldValidate, OP_DEPLOY, OP_MINT, PREFIX_INSCRIPTION,
    PREFIX_INSCRIPTION_HEX,
};
use anyhow::{anyhow, Ok};
use ethers::{
    abi::AbiEncode,
    providers::{Middleware, StreamExt},
    types::{BlockNumber, Transaction},
};

impl Indexer {
    pub async fn index_inscriptions(&self) -> Result<(), anyhow::Error> {
        let (indexed_block, mut block_txi): (i64, i64) =
            self.get_indexed_block(self.indexed_type).await;
        let mut block_to_process = indexed_block as u64;
        let mut block_stream = self.provider.watch_blocks().await?;
        let next_block = |block, _| -> (u64, i64) { (block + 1, -1) };
        while block_stream.next().await.is_some() {
            let block_number = self
                .provider
                .get_block(BlockNumber::Latest)
                .await?
                .unwrap()
                .number
                .unwrap();
            if block_to_process <= block_number.as_u64() {
                let txs = self.provider.get_block_with_txs(block_to_process).await?;
                if let None = txs {
                    (block_to_process, block_txi) = next_block(block_to_process, block_txi);
                    continue;
                }
                let mut txs = txs.unwrap().transactions;
                txs = txs
                    .into_iter()
                    .filter(|tx| tx.transaction_index.unwrap().as_u64() as i64 > block_txi)
                    .collect::<Vec<Transaction>>();
                for tx in txs.iter() {
                    if tx.to.is_none() {
                        let err = anyhow!("Invalid transaction {}: to is None", tx.hash);
                        return Err(err);
                    }
                    if tx.to.unwrap().ne(&tx.from) {
                        continue;
                    }
                    if !tx
                        .input
                        .to_owned()
                        .encode_hex()
                        .starts_with(PREFIX_INSCRIPTION_HEX)
                    {
                        continue;
                    }
                    let input = String::from_utf8(tx.input.to_vec())?;
                    let data = input.strip_prefix(PREFIX_INSCRIPTION).unwrap_or("{}");
                    let deserialized = serde_json::from_str::<serde_json::Value>(data);
                    if deserialized.is_err() {
                        continue;
                    }
                    let deserialized = deserialized.unwrap();
                    if !deserialized.is_object() {
                        continue;
                    }
                    if !deserialized.is_valid_inscription() {
                        continue;
                    }
                    let inscription: Inscription = serde_json::from_value(deserialized)?;
                    let (_, txi) = self.dump_inscription(tx, &inscription).await?;
                    block_txi = txi as i64;
                }
            }
            (block_to_process, block_txi) = next_block(block_to_process, block_txi);
        }
        Ok(())
    }

    async fn dump_inscription(
        &self,
        tx: &Transaction,
        inp: &Inscription,
    ) -> Result<(i64, i64), anyhow::Error> {
        self.database
            ._transaction()
            .run(|db_tx| async move {
                let op = inp.op.as_str();
                if op.eq(OP_DEPLOY) {
                    dump_deploy_inscription(&db_tx, tx, inp).await?;
                }
                if op.eq(OP_MINT) {
                    dump_mint_inscription(&db_tx, tx, inp).await?;
                }
                let indexed_block = tx.block_number.unwrap().as_u64() as i64;
                let indexed_txi = tx.transaction_index.unwrap().as_u64() as i64;
                update_indexed_block(&db_tx, &self.chain, indexed_block, indexed_txi).await?;
                Ok((indexed_block, indexed_txi))
            })
            .await
    }
}