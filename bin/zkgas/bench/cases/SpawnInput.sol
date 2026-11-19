// SPDX-License-Identifier: MIT
pragma solidity 0.8.21;

// Offline workload fixture: each middle frame expands CALL input memory and
// passes the zero-filled range to a leaf that inspects only the fixed header.
contract SpawnInput {
    fallback() external {
        assembly {
            if lt(calldatasize(), 128) { revert(0, 0) }
            let tag := calldataload(0)
            let count := calldataload(32)
            let length := calldataload(64)
            let kind := calldataload(96)
            if or(gt(tag, 2), or(iszero(count), gt(count, 1024))) { revert(0, 0) }
            if or(gt(length, 1048576), mod(length, 32)) { revert(0, 0) }
            if gt(kind, 3) { revert(0, 0) }
            if eq(tag, 2) { stop() }
            if iszero(eq(calldatasize(), 128)) { revert(0, 0) }
            calldatacopy(0, 0, 128)
            if eq(tag, 1) {
                mstore(0, 2)
                let success := 0
                switch kind
                case 0 { success := delegatecall(gas(), address(), 0, add(128, length), 256, 0) }
                case 1 { success := call(gas(), address(), 0, 0, add(128, length), 256, 0) }
                case 2 { success := callcode(gas(), address(), 0, 0, add(128, length), 256, 0) }
                case 3 { success := staticcall(gas(), address(), 0, add(128, length), 256, 0) }
                if or(iszero(success), returndatasize()) { revert(0, 0) }
                return(0, 0)
            }
            mstore(0, 1)
            for { let i := 0 } lt(i, count) { i := add(i, 1) } {
                let success := delegatecall(gas(), address(), 0, 128, 256, 0)
                if or(iszero(success), returndatasize()) { revert(0, 0) }
            }
            mstore(0, count)
            return(0, 32)
        }
    }
}
