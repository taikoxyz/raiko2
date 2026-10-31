// SPDX-License-Identifier: MIT
pragma solidity 0.8.21;

// Finite BLS G1 MSM value probe. Calldata: count/K/K*(x64,y64,scalar32)/expected128.
contract BlsMsmValues {
    fallback() external {
        assembly {
            function demand(condition) {
                if iszero(condition) { revert(0, 0) }
            }
            demand(iszero(lt(calldatasize(), 64)))
            let count := calldataload(0)
            let k := calldataload(32)
            demand(iszero(gt(count, 2048)))
            demand(or(or(eq(k, 1), eq(k, 4)), eq(k, 16)))
            let inputLength := mul(k, 160)
            demand(eq(calldatasize(), add(192, inputLength)))
            let input := 0x100
            calldatacopy(input, 64, inputLength)
            // Every input is word-aligned. Preallocate all separate output memory
            // before the loop, also for the zero-call matched setup controls.
            let output := add(input, inputLength)
            mstore(add(output, 96), 0)
            let expected := add(64, inputLength)
            for { let i := 0 } lt(i, count) { i := add(i, 1) } {
                for { let word := 0 } lt(word, 128) { word := add(word, 32) } {
                    mstore(add(output, word), 0)
                }
                let success := staticcall(gas(), 12, input, inputLength, output, 128)
                demand(success)
                demand(eq(returndatasize(), 128))
                for { let word := 0 } lt(word, 128) { word := add(word, 32) } {
                    demand(eq(mload(add(output, word)), calldataload(add(expected, word))))
                }
            }
            mstore(0, count)
            return(0, 32)
        }
    }
}
