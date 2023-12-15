from web3 import Web3
from web3.contract import ContractFunction, Contract

from conflux.rpc import RpcClient
from conflux.utils import *
from test_framework.contracts import ConfluxTestFrameworkForContract, ZERO_ADDRESS
from test_framework.util import *
from test_framework.mininode import *


class QuickFixTest(ConfluxTestFrameworkForContract):
    def set_test_params(self):
        self.num_nodes = 1
        self.conf_parameters["executive_trace"] = "true"

    def run_test(self):
        contract: Contract = self.cfx_contract("StaticCrossSpace").deploy()
        receipt = contract.functions.run3().cfx_transact(err_msg='Vm reverted, Call Fail')
        print(self.nodes[0].trace_transaction(receipt['transactionHash']))
        # print(contract.functions.run2().cfx_transact())
        # print(contract.functions.run3().cfx_transact())

if __name__ == "__main__":
    QuickFixTest().main()
