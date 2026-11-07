// SPDX-License-Identifier: MIT
pragma solidity 0.8.21;

// Finite BN254 ECMUL capacity probe. Calldata is count/scalar/expectedX/expectedY.
// Every call reuses one input/output allocation and verifies the actual result.
contract PrecompileWorkload {
    fallback() external {
        assembly {
            function demand(condition) {
                if iszero(condition) { revert(0, 0) }
            }

            demand(eq(calldatasize(), 128))
            let count := calldataload(0)
            demand(iszero(gt(count, 4096)))
            let expectedX := calldataload(64)
            let expectedY := calldataload(96)
            demand(or(expectedX, expectedY))

            // G=(1,2). Input ends at 0x160 and therefore allocates
            // the separate output [0x80,0xc0) before the loop, including count=0.
            mstore(0x100, 1)
            mstore(0x120, 2)
            mstore(0x140, calldataload(32))
            mstore(0x80, 0)
            mstore(0xa0, 0)

            for { let i := 0 } lt(i, count) { i := add(i, 1) } {
                // A skipped or short copy must not pass using the previous result.
                mstore(0x80, 0)
                mstore(0xa0, 0)
                let success := staticcall(gas(), 7, 0x100, 96, 0x80, 64)
                demand(success)
                demand(eq(returndatasize(), 64))
                // Expected points come from independent affine scalar multiplication
                // in the producer. Both words must be copied freshly on every call.
                demand(eq(mload(0x80), expectedX))
                demand(eq(mload(0xa0), expectedY))
            }

            mstore(0, count)
            return(0, 32)
        }
    }
}
