//! Block layout: what only a slow path reaches is cold (#1046, docs/decisions.md "Cranelift: cold
//! continuations"). Only flags and jump-only blocks change, so no operation does.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::flowgraph::ControlFlowGraph;
use cranelift_codegen::ir::{self, BlockArg, InstBuilder};

/// Mark a block cold when all its predecessors are, and send a conditional edge from a cold block
/// to a hot one through a cold block of its own (Cranelift gives such an edge's split the coldness
/// of its successor, which puts it in the hot stream).
pub(crate) fn propagate(func: &mut ir::Function) {
    let cfg = ControlFlowGraph::with_function(func);
    let mut work: Vec<ir::Block> = func
        .layout
        .blocks()
        .filter(|&b| func.layout.is_cold(b))
        .collect();
    while let Some(b) = work.pop() {
        for s in cfg.succ_iter(b) {
            if !func.layout.is_cold(s) && cfg.pred_iter(s).all(|p| func.layout.is_cold(p.block)) {
                func.layout.set_cold(s);
                work.push(s);
            }
        }
    }
    let blocks: Vec<ir::Block> = func.layout.blocks().collect();
    for b in blocks {
        let Some(last) = func.layout.last_inst(b).filter(|_| func.layout.is_cold(b)) else {
            continue;
        };
        let dfg = &func.dfg;
        let n = dfg.insts[last]
            .branch_destination(&dfg.jump_tables, &dfg.exception_tables)
            .len();
        for k in (0..n).filter(|_| n > 1) {
            let dfg = &func.dfg;
            let succ = dfg.insts[last].branch_destination(&dfg.jump_tables, &dfg.exception_tables)
                [k]
                .block(&dfg.value_lists);
            if func.layout.is_cold(succ) {
                continue;
            }
            let t = func.dfg.make_block();
            func.layout.append_block(t);
            func.layout.set_cold(t);
            let params = func.dfg.block_params(succ).to_vec();
            let args: Vec<BlockArg> = params
                .into_iter()
                .map(|v| {
                    let ty = func.dfg.value_type(v);
                    BlockArg::Value(func.dfg.append_block_param(t, ty))
                })
                .collect();
            FuncCursor::new(func).at_bottom(t).ins().jump(succ, &args);
            let dfg = &mut func.dfg;
            dfg.insts[last].branch_destination_mut(&mut dfg.jump_tables, &mut dfg.exception_tables)
                [k]
                .set_block(t, &mut dfg.value_lists);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift_codegen::ir::types::I64;
    use cranelift_codegen::settings;
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

    /// entry: brif ok, slow. slow (cold): brif cont, join(0). cont: jump join(1). ok: jump join(2).
    #[test]
    fn continuation_goes_cold_and_a_cold_edge_to_a_hot_join_gets_its_own_block() {
        let mut func = ir::Function::new();
        func.signature.params.push(ir::AbiParam::new(I64));
        func.signature.returns.push(ir::AbiParam::new(I64));
        let mut fc = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut func, &mut fc);
        let [entry, slow, cont, ok, join] = [(); 5].map(|()| b.create_block());
        b.append_block_params_for_function_params(entry);
        b.append_block_param(join, I64);
        b.set_cold_block(slow);
        b.switch_to_block(entry);
        let x = b.block_params(entry)[0];
        b.ins().brif(x, ok, &[], slow, &[]);
        b.switch_to_block(slow);
        let zero = b.ins().iconst(I64, 0);
        b.ins().brif(x, cont, &[], join, &[BlockArg::Value(zero)]);
        b.switch_to_block(cont);
        let one = b.ins().iconst(I64, 1);
        b.ins().jump(join, &[BlockArg::Value(one)]);
        b.switch_to_block(ok);
        let two = b.ins().iconst(I64, 2);
        b.ins().jump(join, &[BlockArg::Value(two)]);
        b.switch_to_block(join);
        let r = b.block_params(join)[0];
        b.ins().return_(&[r]);
        b.seal_all_blocks();
        let flags = settings::Flags::new(settings::builder());
        let isa = cranelift_native::builder()
            .expect("host")
            .finish(flags.clone())
            .expect("host");
        b.finalize(isa.frontend_config());

        let before = func.layout.blocks().count();
        propagate(&mut func);
        cranelift_codegen::verify_function(&func, &flags).expect("still valid");
        assert!(func.layout.is_cold(cont), "continuation of the slow path");
        assert!(!func.layout.is_cold(ok) && !func.layout.is_cold(join));
        assert!(!func.layout.is_cold(entry));
        assert_eq!(func.layout.blocks().count(), before + 1);
        let t = func.layout.last_block().unwrap();
        assert!(func.layout.is_cold(t));
        let last = func.layout.last_inst(slow).unwrap();
        let dfg = &func.dfg;
        let dests: Vec<_> = dfg.insts[last]
            .branch_destination(&dfg.jump_tables, &dfg.exception_tables)
            .iter()
            .map(|d| d.block(&dfg.value_lists))
            .collect();
        assert_eq!(dests, [cont, t], "the edge to the join goes through it");
    }
}
