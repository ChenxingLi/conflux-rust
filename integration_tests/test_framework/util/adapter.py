from typing import Optional, Union

from web3 import Web3
from web3._utils.transactions import fill_nonce, fill_transaction_defaults
from eth_account.signers.local import LocalAccount
from ethereum_test_types import EOA
from ethereum_test_tools import (
    Address,
    Account,
    Block,
    Bytecode,
    Environment,
    Initcode,
    Storage,
    Transaction,
    Opcodes as Op,
    EVMCodeType,
)
from ethereum_test_base_types import StorageRootType
from integration_tests.test_framework.util.eip7702.eip7702 import (
    sign_authorization,
    send_eip7702_transaction,
    sign_eip7702_transaction_with_default_fields,
    Authorization,
)

from integration_tests.test_framework.test_framework import ConfluxTestFramework

class AllocMock:
    def __init__(self, ew3: Web3, genesis_account: LocalAccount):
        self.ew3 = ew3
        self.genesis_account = genesis_account
    
    def fund_eoa(self, amount: Optional[int] = None, delegation: Optional[Address] = None) -> EOA:
        if amount is None:
            amount = self.ew3.to_wei(1, "ether")
        new_account = self.ew3.eth.account.create()
        if amount > 0:
            tx_hash = self.ew3.eth.send_transaction(
                {
                    "from": self.genesis_account.address,
                    "to": new_account.address,
                    "value": amount,
                }
            )
            self.ew3.eth.wait_for_transaction_receipt(tx_hash)
        if delegation is None:
            return EOA(key=new_account.key)
        tx_hash = send_eip7702_transaction(
            self.ew3,
            self.genesis_account,
            {
                "authorizationList": [
                    sign_authorization(
                        contract_address=str(delegation),
                        chain_id=self.ew3.eth.chain_id,
                        nonce=0,
                        private_key=new_account.key.to_0x_hex(),
                    )
                ],
                "to": "0x0000000000000000000000000000000000000000",
            }
        )
        self.ew3.eth.wait_for_transaction_receipt(tx_hash)
        return EOA(key=new_account.key, nonce=1)
    
    def deploy_contract(self, code: Bytecode, *, balance: int = 0, storage: Union[Storage, StorageRootType, None] = None, evm_code_type: EVMCodeType = EVMCodeType.LEGACY) -> Address:
        initcode_prefix = Bytecode()
        if evm_code_type != EVMCodeType.LEGACY:
            raise NotImplementedError("Only legacy code type is supported for now")
        if storage is None:
            storage = {}
        if not isinstance(storage, Storage):
            storage = Storage(storage)  # type: ignore
        if storage is not None and len(storage.root) > 0:
            initcode_prefix += sum(Op.SSTORE(key, value) for key, value in storage.root.items())
        initcode = Initcode(deploy_code=code, initcode_prefix=initcode_prefix)
        tx_hash = self.ew3.eth.send_transaction(
            {
                "from": self.genesis_account.address,
                "data": bytes(initcode),
                "value": balance,
            }
        )
        receipt = self.ew3.eth.wait_for_transaction_receipt(tx_hash)
        return Address(receipt["contractAddress"])


def conflux_state_test(
    ew3: Web3, 
    network: ConfluxTestFramework,
    env: Environment,
    pre: AllocMock,
    post: dict[Address, Account],
    genesis_environment = None,
    tx: Optional[Transaction] = None,
    blocks: Optional[list[Block]] = None,
    t8n_dump_dir=None,  # Optional parameter
):
    def get_raw_tx_from_transaction(tx: Transaction) -> bytes:
        if tx.authorization_list is not None:
        # Send transaction
            raw_tx = sign_eip7702_transaction_with_default_fields(
                ew3, 
                ew3.eth.account.from_key(tx.sender.key),  # type: ignore
                {
                    "nonce": tx.nonce,
                    "value": tx.value,
                    "to": tx.to.hex() if tx.to is not None else None,
                    "gas": tx.gas_limit,
                    "data": tx.data.hex() if tx.data is not None else None,
                    "authorizationList": [
                        Authorization(
                            contract_address=str(auth.address),
                            chain_id=auth.chain_id,
                            nonce=int(auth.nonce),
                            r=hex(auth.r),
                            s=hex(auth.s),
                            v=auth.v,
                            yParity=auth.v
                        ) for auth in tx.authorization_list 
                    ] if tx.authorization_list is not None else None
                }  # type: ignore
            )
            return raw_tx
        tx_to_send = {
            "from": tx.sender,
            "nonce": tx.nonce,
            "value": tx.value,
            "to": tx.to,
            "gas": tx.gas_limit,
            "data": tx.data,
        }
        tx_to_sign = fill_transaction_defaults(ew3, fill_nonce(ew3, tx_to_send))
        raw_tx = ew3.eth.account.sign_transaction(tx_to_sign, tx.sender.key).raw_transaction
        return raw_tx
        
    if not tx and not blocks:
        raise ValueError("tx or block must be provided")
    
    if tx and blocks:
        raise ValueError("tx and blocks cannot both be provided")
    
    if blocks:
        current_block = network.client.best_block_hash()
        for block in blocks:
            tx_list = [get_raw_tx_from_transaction(tx) for tx in block.txs]
            block_hash = network.client.generate_custom_block(current_block, [], txs=tx_list)
            current_block = block_hash
        network.client.generate_blocks(4, num_txs=1)
        block = ew3.eth.get_block(current_block, True)
        txs = block["transactions"]
    else:
        raw_tx = get_raw_tx_from_transaction(tx)
        tx_hash = ew3.eth.send_raw_transaction(raw_tx)
        receipt = ew3.eth.wait_for_transaction_receipt(tx_hash, timeout=1, poll_latency=0.5)
        if receipt["status"] == 0:
            print(f"Transaction failed: {tx_hash.hex()}")
            print(f"TxErrorMsg: {receipt.get('txErrorMsg', 'No error message')}")

    # Check post-conditions
    # Collect all state differences instead of breaking on first failure
    differences = []
    
    for address, expected_account in post.items():
        # Convert Address to string for web3 compatibility
        address_str = ew3.to_checksum_address(address)

        # Handle non-existent accounts
        if expected_account is None:
            try:
                actual_nonce = ew3.eth.get_transaction_count(address_str)
                if actual_nonce != 0:
                    differences.append(f"Account {address} nonce should be 0, actual: {actual_nonce}")
            except Exception as e:
                differences.append(f"Error checking nonce for account {address}: {str(e)}")
            
            try:
                actual_code = ew3.eth.get_code(address_str)
                if actual_code != b'':
                    differences.append(f"Account {address} code should be empty, actual: {actual_code.hex()}")
            except Exception as e:
                differences.append(f"Error checking code for account {address}: {str(e)}")
            
            try:
                actual_balance = ew3.eth.get_balance(address_str)
                if actual_balance != 0:
                    differences.append(f"Account {address} balance should be 0, actual: {actual_balance}")
            except Exception as e:
                differences.append(f"Error checking balance for account {address}: {str(e)}")
            
            continue
        
        # Check nonce
        if expected_account.nonce != 0:
            try:
                actual_nonce = ew3.eth.get_transaction_count(address_str)
                if actual_nonce != expected_account.nonce:
                    differences.append(f"Account {address} nonce mismatch: expected={expected_account.nonce}, actual={actual_nonce}")
            except Exception as e:
                differences.append(f"Error checking nonce for account {address}: {str(e)}")
        
        # Check code
        if expected_account.code != b'':
            try:
                actual_code = ew3.eth.get_code(address_str)
                if actual_code != expected_account.code:
                    differences.append(f"Account {address} code mismatch: expected length={len(expected_account.code)}, actual length={len(actual_code)}")
                    if len(actual_code) < 100 and len(expected_account.code) < 100:
                        differences.append(f"  Expected code: {expected_account.code.hex()}")
                        differences.append(f"  Actual code: {actual_code.hex()}")
            except Exception as e:
                differences.append(f"Error checking code for account {address}: {str(e)}")
        
        # Check storage
        if hasattr(expected_account, 'storage') and expected_account.storage is not None:
            for key, expected_value in expected_account.storage.items():
                try:
                    actual_value = int(ew3.eth.get_storage_at(address_str, key).hex(), 16)
                    if actual_value != expected_value:
                        differences.append(f"Account {address} storage slot {key} mismatch: expected={expected_value}, actual={actual_value}")
                except Exception as e:
                    differences.append(f"Error checking storage slot {key} for account {address}: {str(e)}")
    
    # If differences found, display in table format and raise exception
    if differences:
        error_message = "\nState validation failed, found the following differences:\n" + "\n".join(differences)
        
        # Add comprehensive state differences table
        error_message += "\n\nState Differences Table:\n"
        error_message += "+----------------+----------+------------------+------------------+\n"
        error_message += "| Address        | Type     | Expected         | Actual           |\n"
        error_message += "+----------------+----------+------------------+------------------+\n"
        
        for address, expected_account in post.items():
            address_str = ew3.to_checksum_address(address)
            addr_short = f"{str(address_str)[:6]}...{str(address_str)[-4:]}"
            
            if expected_account is None:
                # Non-existent account
                error_message += f"| {addr_short:<14} | existence | Non-existent      | "
                try:
                    actual_nonce = ew3.eth.get_transaction_count(address_str)
                    actual_code = ew3.eth.get_code(address_str)
                    actual_balance = ew3.eth.get_balance(address_str)
                    if actual_nonce == 0 and actual_code == b'' and actual_balance == 0:
                        error_message += "Default state     |\n"
                    else:
                        error_message += "Exists            |\n"
                except Exception as e:
                    error_message += f"Check failed      |\n"
            else:
                # Check nonce
                if expected_account.nonce != 0:
                    error_message += f"| {addr_short:<14} | nonce    | {expected_account.nonce:<16} | "
                    try:
                        actual_nonce = ew3.eth.get_transaction_count(address_str)
                        error_message += f"{actual_nonce:<16} |\n"
                    except Exception as e:
                        error_message += f"Check failed      |\n"
                
                # Check code
                if expected_account.code != b'':
                    error_message += f"| {addr_short:<14} | code     | {len(expected_account.code)} bytes         | "
                    try:
                        actual_code = ew3.eth.get_code(address_str)
                        error_message += f"{len(actual_code)} bytes         |\n"
                    except Exception as e:
                        error_message += f"Check failed      |\n"
                
                # Check balance if specified
                if hasattr(expected_account, 'balance') and expected_account.balance != 0:
                    error_message += f"| {addr_short:<14} | balance  | {expected_account.balance:<16} | "
                    try:
                        actual_balance = ew3.eth.get_balance(address_str)
                        error_message += f"{actual_balance:<16} |\n"
                    except Exception as e:
                        error_message += f"Check failed      |\n"
                
                # Check storage
                if hasattr(expected_account, 'storage') and expected_account.storage is not None:
                    for key, expected_value in expected_account.storage.items():
                        expected_hex = f"0x{expected_value:x}"
                        error_message += f"| {addr_short:<14} | slot {key:<4} | {expected_hex:<16} | "
                        try:
                            actual_value = int(ew3.eth.get_storage_at(address_str, key).hex(), 16)
                            actual_hex = f"0x{actual_value:x}"
                            error_message += f"{actual_hex:<16} |\n"
                        except Exception as e:
                            error_message += f"Check failed      |\n"
        
        error_message += "+----------------+----------+------------------+------------------+"
        
        raise AssertionError(error_message)
