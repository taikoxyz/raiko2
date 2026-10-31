// SPDX-License-Identifier: MIT
pragma solidity 0.8.21;

// Finite MODEXP value-axis probe. Calldata is count/width/raw input/expected output.
// One input allocation and separate output allocation are reused for every call.
contract ModexpValues {
    fallback() external {
        assembly {
            function demand(condition) {
                if iszero(condition) { revert(0, 0) }
            }

            demand(iszero(lt(calldatasize(), 64)))
            let count := calldataload(0)
            let width := calldataload(32)
            demand(iszero(gt(count, 65536)))
            demand(or(or(eq(width, 128), eq(width, 256)), eq(width, 512)))
            demand(eq(calldatasize(), add(161, mul(3, width))))
            demand(eq(calldataload(64), width))
            demand(eq(calldataload(96), 1))
            demand(eq(calldataload(128), width))

            let inputLength := add(97, mul(2, width))
            let input := 0x100
            calldatacopy(input, 64, inputLength)
            // Round past the complete input, with no input/output overlap. Allocate
            // all output memory even for count=0, retaining a matched setup control.
            let output := and(add(add(input, inputLength), 31), not(31))
            mstore(sub(add(output, width), 32), 0)
            let expected := add(64, inputLength)
            for { let i := 0 } lt(i, count) { i := add(i, 1) } {
                for { let word := 0 } lt(word, width) { word := add(word, 32) } {
                    mstore(add(output, word), 0)
                }
                let success := staticcall(gas(), 5, input, inputLength, output, width)
                demand(success)
                demand(eq(returndatasize(), width))
                for { let word := 0 } lt(word, width) { word := add(word, 32) } {
                    demand(eq(mload(add(output, word)), calldataload(add(expected, word))))
                }
            }
            mstore(0, count)
            return(0, 32)
        }
    }
}
