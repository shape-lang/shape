use super::*;

impl VirtualMachine {
    pub(super) fn blob_hash_for_function(&self, func_id: u16) -> Option<FunctionHash> {
        self.function_hashes
            .get(func_id as usize)
            .copied()
            .flatten()
    }

    #[inline]
    pub(super) fn function_id_for_blob_hash(&self, hash: FunctionHash) -> Option<u16> {
        self.function_id_by_hash.get(&hash).copied()
    }

    pub(super) fn current_locals_base(&self) -> usize {
        self.call_stack
            .last()
            .map(|frame| frame.base_pointer)
            .unwrap_or(0)
    }

    // call_function_with_nb_args(), call_closure_with_nb_args(),
    // call_value_immediate_nb(), call_function_from_stack()
    // moved to call_convention module.

    // handle_eval_datetime_expr(), handle_window_functions(), handle_join_execute(),
    // exec_bind_schema(), exec_load_col() moved to window_join module.

    // execute_until_call_depth(), execute_fast(), execute_instruction()
    // moved to dispatch module.
}
