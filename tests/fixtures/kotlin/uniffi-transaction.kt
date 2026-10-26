open class BknDbTransaction: Disposable, AutoCloseable, BknDbTransactionInterface
{
    /**
     */
    constructor(withHandle: UniffiWithHandle, handle: Long) {
    }
    /**
     */
    override fun destroy() {
    }
    internal inline fun <R> callWithHandle(block: (handle: Long) -> R): R {
        do {
        } while (! this.callCounter.compareAndSet(c, c + 1L))
        try {
        } finally {
        }
    }
    @Throws(FfiBknException::class)override fun `getEdge`(`id`: kotlin.ULong): FfiEdgeRecord? {
            return FfiConverterOptionalTypeFfiEdgeRecord.lift(
    callWithHandle {
    }
    )
    }
    @Throws(FfiBknException::class)override fun `insert`(`table`: kotlin.String, `values`: Map<kotlin.String, FfiPropValue>): FfiPropValue {
    }
    @Throws(FfiBknException::class)override fun `insertMany`(`table`: kotlin.String, `rows`: List<Map<kotlin.String, FfiPropValue>>): List<FfiPropValue> {
            return FfiConverterSequenceTypeFfiPropValue.lift(
    callWithHandle {
    uniffiRustCallWithError(FfiBknException) { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_insert_many(
        FfiConverterSequenceMapStringTypeFfiPropValue.lower(`rows`),_status)
}
    }
    )
    }
    /**
     */override fun `isActive`(): kotlin.Boolean {
            return FfiConverterBoolean.lift(
    callWithHandle {
    uniffiRustCall() { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_is_active(
        _status)
}
    }
    )
    }
    @Throws(FfiBknException::class)override fun `neighbors`(`node`: kotlin.ULong, `direction`: FfiDirection, `edgeType`: kotlin.String?): List<FfiTypedNeighbor> {
            return FfiConverterSequenceTypeFfiTypedNeighbor.lift(
    callWithHandle {
    uniffiRustCallWithError(FfiBknException) { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_neighbors(
        FfiConverterOptionalString.lower(`edgeType`),_status)
}
    }
    )
    }
    @Throws(FfiBknException::class)override fun `rollback`()
        = 
    callWithHandle {
    uniffiRustCallWithError(FfiBknException) { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_rollback(
        _status)
}
    }
    @Throws(FfiBknException::class)override fun `select`(`table`: kotlin.String, `query`: FfiQuery): List<FfiRow> {
            return FfiConverterSequenceTypeFfiRow.lift(
    callWithHandle {
    uniffiRustCallWithError(FfiBknException) { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_select(
        FfiConverterTypeFfiQuery.lower(`query`),_status)
}
    }
    )
    }
    @Throws(FfiBknException::class)override fun `sql`(`query`: kotlin.String, `positional`: List<FfiPropValue>, `named`: Map<kotlin.String, FfiPropValue>?): FfiQueryResult {
            return FfiConverterTypeFfiQueryResult.lift(
    callWithHandle {
    uniffiRustCallWithError(FfiBknException) { _status ->
    UniffiLib.uniffi_bkndb_ffi_fn_method_bkndbtransaction_sql(
        FfiConverterOptionalMapStringTypeFfiPropValue.lower(`named`),_status)
}
    }
    )
    }
}
