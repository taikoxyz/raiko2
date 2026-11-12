// SPDX-License-Identifier: MIT
pragma solidity 0.8.21;

// Distinguish automatic CALL returndata transport from caller memory expansion.
// Outer calldata is always eight words; only the output-capacity word changes
// within a zero/half/full triplet. The parent reserves input/output memory once.
contract ReturnCopy {
    fallback() external {
        assembly {
            function demand(condition) {
                if iszero(condition) { revert(0, 0) }
            }
            function markPayload(start, length, marker) {
                if length { mstore8(start, marker) }
                if gt(length, 1) { mstore8(add(start, sub(length, 1)), 0x5a) }
            }
            function checkOutput(start, length, capacity, marker) {
                let sentinel := 0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
                let rounded := and(add(length, 31), not(31))
                demand(eq(mload(sub(start, 32)), sentinel))
                demand(eq(mload(add(start, rounded)), sentinel))
                demand(eq(byte(0, mload(add(start, length))), 0xcc))
                // Every mode evaluates every load/check. Boolean masks make the
                // relevant expectation conditional without skipping opcode work.
                demand(or(iszero(capacity), eq(byte(0, mload(start)), marker)))
                let expectedLast := add(mul(eq(capacity, 1), marker),
                    mul(and(gt(length, 1), eq(capacity, length)), 0x5a))
                let lastCopy := sub(add(capacity, iszero(capacity)), 1)
                demand(or(iszero(capacity),
                    eq(byte(0, mload(add(start, lastCopy))), expectedLast)))
                demand(or(iszero(iszero(capacity)), eq(byte(0, mload(start)), 0xcc)))
                demand(or(iszero(lt(capacity, length)),
                    eq(byte(0, mload(add(start, capacity))), 0xcc)))
                let lastData := sub(add(length, iszero(length)), 1)
                demand(or(iszero(lt(capacity, length)),
                    eq(byte(0, mload(add(start, lastData))), 0xcc)))
            }

            demand(eq(calldatasize(), 256))
            let tag := calldataload(0)
            let length := calldataload(64)
            let childRevert := calldataload(192)
            demand(iszero(gt(length, 1048576)))
            demand(iszero(gt(childRevert, 1)))
            if eq(tag, 1) {
                markPayload(0, length, xor(0xa5, and(calldataload(224), 1)))
                if childRevert { revert(0, length) }
                return(0, length)
            }
            demand(iszero(tag))
            let count := calldataload(32)
            let capacity := calldataload(96)
            let targetKind := calldataload(128)
            let callKind := calldataload(160)
            demand(and(iszero(iszero(count)), iszero(gt(count, 1024))))
            demand(iszero(gt(capacity, length)))
            demand(iszero(gt(targetKind, 1)))
            demand(iszero(gt(callKind, 3)))
            demand(iszero(and(targetKind, childRevert)))
            demand(iszero(calldataload(224)))

            // Header [0x100,0x200), payload [0x240,0x240+length), then two
            // separation words and distinct output/padding/suffix-guard words.
            // Reserve the same complete ranges for every output-capacity mode.
            calldatacopy(0x100, 0, 256)
            mstore(0x100, 1)
            let rounded := and(add(length, 31), not(31))
            let output := add(0x280, rounded)
            mstore(add(0x240, length), 0)
            let sentinel := 0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
            mstore(sub(output, 32), sentinel)
            for { let position := output } lt(position, add(output, add(rounded, 32)))
                { position := add(position, 32) } { mstore(position, sentinel) }
            markPayload(0x240, length, 0xa5)

            for { let i := 0 } lt(i, count) { i := add(i, 1) } {
                let marker := xor(0xa5, and(i, 1))
                mstore(add(0x100, 224), i)
                if length { mstore8(0x240, marker) }
                let destination := address()
                let inputOffset := 0x100
                let inputLength := 256
                if targetKind {
                    destination := 4
                    inputOffset := 0x240
                    inputLength := length
                }
                let success := 0
                switch callKind
                case 0 { success := delegatecall(gas(), destination, inputOffset, inputLength, output, capacity) }
                case 1 { success := call(gas(), destination, 0, inputOffset, inputLength, output, capacity) }
                case 2 { success := callcode(gas(), destination, 0, inputOffset, inputLength, output, capacity) }
                case 3 { success := staticcall(gas(), destination, inputOffset, inputLength, output, capacity) }
                demand(eq(success, iszero(childRevert)))
                demand(eq(returndatasize(), length))
                checkOutput(output, length, capacity, marker)
            }
            mstore(0, count)
            return(0, 32)
        }
    }
}
