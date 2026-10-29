//! The differential corpus through sem, lowering, the IR verifier, the interpreter and the
//! kernel, compared with the sim1 reference outputs (`tests/corpus/**/*.expected`, canonical
//! form of `tools/difftest/README.md`).
//!
//! Each file is elaborated with yavs-elab (which lowers through this crate) and run on the
//! kernel, the same path as `yavs run`. Files listed in `PASS` must match their `.expected`
//! exactly; the others are printed with their difference or diagnostic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yavs_diag::{MessageFormat, render};
use yavs_kernel::{Config, EndReason, Hook, Observer, Report, RunStatus, Severity, Time};
use yavs_lower::{LowerOptions, Lowerer};
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, LangRev, Options};

/// Corpus files whose output matches sim1's exactly.
const PASS: &[&str] = &[
    "protected/access_method_call_2019",
    "textio/file_in_subprogram",
    "textio/read_failed",
    "textio/read_bit_vector_white_space",
    "textio/binary_file_composites",
    "textio/binary_file_bounds",
    "access/access_string_array",
    "access/gc_random",
    "access/linked_list",
    "access/allocator_overload",
    "access/null_deref",
    "access/package_alloc",
    "arith/div_zero",
    "arith/int_ops",
    "arith/int_overflow",
    "arith/real_arith",
    "arith/real_to_int_range",
    "arith/time_arith",
    "composite/alias_slice_call_constant",
    "composite/sigrec_whole",
    "composite/sigrec_whole_generic",
    "composite/record_pack_signals",
    "composite/record_pack_ports",
    "composite/sig_array_part",
    "composite/record_conversion_2019",
    "composite/record_subtype_others",
    "composite/others_element_attr",
    "composite/case_aggregate_choice",
    "composite/process_label_subtype",
    "composite/user_attribute_index",
    "composite/attribute_value_subtype",
    "composite/value_composite_2019",
    "composite/record_constant_element_bounds",
    "composite/alias_subtype_open_elements",
    "generics/package_instance_deferred",
    "generics/package_instance_deferred_call",
    "composite/subtype_attr_conversion",
    "composite/others_unconstrained_target",
    "sigassign/slice_param_range",
    "composite/static_slice_choice",
    "composite/narrow_int_storage",
    "composite/layout_memo_lengths",
    "scalar/slot_promotion",
    "composite/logic_fill",
    "composite/loop_invariant",
    "composite/loop_invariant_effect",
    "composite/loop_invariant_fault_order",
    "composite/loop_invariant_replay",
    "composite/process_loop_invariant",
    "composite/process_instance_slices",
    "composite/alias_constant",
    "composite/record_aggregate_target",
    "composite/case_slice_choice",
    "composite/case_length_mismatch",
    "composite/record_aggregate_field_length",
    "composite/record_aggregate_field_range",
    "composite/aggregate_target_logic_fields",
    "composite/aggregate_target_matrix",
    "composite/aggregate_target_matrix_cover",
    "composite/aggregate_target_named_record",
    "composite/aggregate_target_nested_array",
    "composite/aggregate_target_nested_record",
    "composite/aggregate_named_pieces",
    "composite/aggregate_piece_direction",
    "composite/aggregate_open_elements",
    "composite/aggregate_2d_named_rows",
    "composite/aggregate_fill",
    "composite/periodic_const",
    "composite/element_attr_open",
    "composite/element_depth",
    "composite/element_open_formal",
    "composite/record_port_bounds",
    "composite/alias_object_subtype",
    "composite/agg_port_range_choice",
    "composite/record_in_port_bounds",
    "composite/element_open_formal_mismatch",
    "composite/port_length_const",
    "composite/aggregate_pieces",
    "composite/aggregate_target_cover",
    "composite/aggregate_target_cover_signal",
    "composite/aggregate_target_dynamic",
    "composite/aggregate_target_dynamic_cover",
    "composite/aggregate_target_elems",
    "composite/aggregate_target_elems_cover",
    "composite/aggregate_target_elems_order",
    "composite/aggregate_element_lengths",
    "composite/aggregate_target_range_length",
    "composite/aggregate_target_range_order",
    "composite/aggregate_target_ranges",
    "composite/aggregate_target_ranges_signal",
    "composite/aggregate_target_self",
    "composite/array_records",
    "composite/index_constraint_compat",
    "composite/index_constraint_attr",
    "composite/index_constraint_elem",
    "composite/array_conversion_index",
    "composite/nested_alias_slices",
    "composite/nested_selections",
    "composite/arrays_aggregates",
    "composite/concat_composite",
    "composite/constant_bounds",
    "composite/package_record_signals",
    "composite/range_value",
    "composite/record_signal_attrs",
    "composite/record_signal_params",
    "composite/records",
    "composite/signal_bounds_init",
    "composite/signal_composite_bounds",
    "composite/slv_register",
    "composite/strings_ops",
    "composite/unconstrained_records",
    "composite/record_signal_field_read",
    "composite/record_signal_vector_read",
    "delta/async_triggers",
    "delta/concurrent_assign",
    "delta/delta_chain",
    "delta/delta_order",
    "delta/event_attrs",
    "delta/guard_initial_value",
    "delta/signal_update_order",
    "delta/skip_unchanged",
    "delta/wait_edge",
    "env/assert_clear_exit",
    "env/file_ops_2019",
    "env/assert_count_cycles",
    "env/assert_count",
    "env/call_path",
    "env/call_path_thin",
    "env/finish_in_function_call_order",
    "env/finish_noarg",
    "env/finish_status",
    "env/stop_status",
    "env/stop_time_limit",
    "env/time_records",
    "force/ext_force_out_port",
    "force/force_composite",
    "force/force_composite_ports",
    "force/force_drivers",
    "force/force_external_composite",
    "force/force_in",
    "force/force_out_port",
    "force/force_resolved_composite",
    "force/force_selected",
    "generics/call_generic_map_2019",
    "generics/call_generic_map_2019_hand",
    "generics/package_generic_map",
    "generics/generic_nested_proc",
    "generics/config_generic_types",
    "generics/case_generate_array",
    "generics/attr_image_value",
    "generics/composite_eval",
    "generics/deferred_constant",
    "generics/elab_block_function",
    "generics/elab_calls",
    "generics/elab_values",
    "generics/for_generate",
    "generics/generic_blocks_2019",
    "generics/generic_components_2019",
    "generics/generic_entities_2019",
    "generics/generic_entity_locals_2019",
    "generics/generic_map",
    "generics/generic_constrained_bounds",
    "generics/generic_map_signature_2019",
    "generics/generic_subprogram_default_2019",
    "generics/generic_types_2019",
    "generics/if_case_generate",
    "generics/local_generic_packages",
    "generics/local_generic_packages_hand",
    "generics/package_generics",
    "generics/formal_package_subprograms",
    "generics/formal_packages",
    "generics/named_aggregate_direction",
    "generics/nested_generic_packages",
    "generics/nested_packages",
    "generics/pkg_generic_kinds",
    "generics/pkg_generic_local_state",
    "generics/pkg_generic_state",
    "generics/pkg_generic_state_2019",
    "generics/pkg_generics_2019",
    "generics/pkg_instance_local",
    "generics/pkg_instance_regions",
    "generics/subprog_instance_overload",
    "generics/subprog_instance_overload_2019",
    "generics/subprog_header_map_2019",
    "generics/unelaborated_generate_constant",
    "hierarchy/entity_statements",
    "hierarchy/type_bounds_once",
    "hierarchy/label_path_name",
    "hierarchy/instance_name_generic",
    "hierarchy/component_binding",
    "hierarchy/config_block_component",
    "hierarchy/config_generate_range_name",
    "hierarchy/package_body_item_name",
    "hierarchy/package_subprogram_param_names",
    "hierarchy/subprogram_signature_names",
    "hierarchy/port_actual_generic_bounds",
    "hierarchy/use_library_all",
    "hierarchy/binding_actual_names",
    "hierarchy/config_generate_index",
    "hierarchy/config_generate_range",
    "hierarchy/constant_reads_signal",
    "hierarchy/config_port_expr",
    "hierarchy/port_init_read",
    "hierarchy/out_port_collapse",
    "hierarchy/driving_value_no_driver",
    "hierarchy/use_type_operations",
    "protected/region_shared_elab",
    "hierarchy/use_type_functions",
    "hierarchy/external_alias",
    "hierarchy/external_direct",
    "hierarchy/external_port_inertial",
    "hierarchy/external_subprogram",
    "hierarchy/external_subprogram_region",
    "hierarchy/external_subprogram_variable",
    "hierarchy/external_variable",
    "hierarchy/incremental_binding",
    "hierarchy/inertial_port_actual",
    "hierarchy/inertial_port_parts",
    "hierarchy/instance_path_names",
    "hierarchy/path_name_generic_actual",
    "hierarchy/instance_process_order",
    "hierarchy/process_local_names",
    "hierarchy/mode_view_generic",
    "hierarchy/mode_view_nested",
    "hierarchy/mode_view_param_array",
    "hierarchy/mode_view_param_nested",
    "hierarchy/mode_view_param_procs",
    "hierarchy/mode_view_ports",
    "hierarchy/mode_view_concurrent_call",
    "hierarchy/port_operator_call_syntax",
    "hierarchy/mode_view_assoc_array",
    "hierarchy/mode_view_assoc_param",
    "hierarchy/mode_view_assoc_param_deep",
    "hierarchy/mode_view_assoc_port",
    "hierarchy/mode_view_signal_parameter",
    "hierarchy/nested_instances",
    "hierarchy/open_port_default",
    "hierarchy/open_port_default_2019",
    "hierarchy/package_const_once",
    "hierarchy/package_shared",
    "hierarchy/package_signal",
    "hierarchy/package_signal_alias",
    "hierarchy/port_conv_composite",
    "hierarchy/port_conv_inout",
    "hierarchy/part_conversion_actual",
    "hierarchy/port_conv_inout_formal_only",
    "hierarchy/port_conv_inout_formal_only_composite",
    "hierarchy/port_conv_record",
    "hierarchy/port_conv_type_composite",
    "hierarchy/port_conv_type_unconstrained",
    "hierarchy/port_expr_actual",
    "hierarchy/port_expr_generic_range",
    "hierarchy/port_expr_part_range",
    "hierarchy/port_expr_part_range_elem",
    "hierarchy/port_expr_parts",
    "hierarchy/port_expr_record",
    "hierarchy/port_expr_record_array",
    "hierarchy/port_expr_unconstrained",
    "hierarchy/port_individual_unconstrained",
    "hierarchy/port_modes_conversion",
    "hierarchy/record_port_parts_nested",
    "hierarchy/record_port_parts_open",
    "hierarchy/use_work_all",
    "ieee/fixed_pkg_ops",
    "ieee/fixed_pkg_ops_2019",
    "ieee/float_pkg_ops",
    "ieee/float_pkg_ops_2019",
    "ieee/ieee_textio",
    "ieee/ieee_textio_2019",
    "ieee/ieee_warnings",
    "ieee/math_complex_ops",
    "ieee/math_complex_ops_2019",
    "ieee/math_real_ops",
    "ieee/math_real_ops_2019",
    "ieee/vital_model",
    "ieee/vital_model_2019",
    "ieee/numeric_bit_ops",
    "ieee/numeric_bit_ops_2019",
    "ieee/numeric_std_ops",
    "ieee/numeric_std_ops_2019",
    "ieee/std_logic_1164_ops",
    "ieee/std_logic_1164_ops_2019",
    "ieee/user_reduce_idiom",
    "postponed/postponed_basic",
    "postponed/postponed_wait",
    "process/all_local_subprogram",
    "process/call_chain",
    "process/call_chain_error",
    "process/call_conversions",
    "process/call_error_path",
    "process/call_conversions_unconstrained",
    "process/case_if",
    "process/no_wait_infinite",
    "process/subtype_bounds_once",
    "process/wait_in_sensitivity_proc",
    "process/conditional_expr_2019",
    "process/edge_shapes",
    "process/individual_assoc",
    "process/individual_assoc_parts",
    "process/process_order",
    "process/signal_param_parts",
    "process/signal_param_records",
    "process/signal_param_slices",
    "process/local_procedure_drivers",
    "process/loops_exit_next",
    "process/named_return_2019",
    "process/named_return_generic_2019",
    "process/named_return_record_2019",
    "process/named_return_scalar_2019",
    "process/return_when_2019",
    "process/sensitivity",
    "process/static_waits",
    "process/subprograms",
    "process/wait_past_time_high",
    "process/unaffected_2019",
    "process/wait_forms",
    "protected/alloc_call_in_init",
    "protected/alloc_call_in_pkg_init",
    "protected/alloc_in_init",
    "protected/composites_2019_hand",
    "protected/counter",
    "protected/expanded_name_in_body",
    "protected/file_in_body",
    "protected/generic_package_prot",
    "protected/impure_const_once",
    "protected/method_in_init",
    "protected/method_alias_objects",
    "protected/method_nested",
    "protected/null_in_table",
    "protected/pointer_queue",
    "protected/private_variables",
    "protected/shared_var_regions",
    "protected/stack",
    "protected/variable_ports",
    "protected/variable_ports_same_delta",
    "reflect/access_file_prot",
    "reflect/arrays",
    "reflect/arrays2",
    "reflect/enums",
    "reflect/generics",
    "reflect/index_bases",
    "reflect/multidim",
    "reflect/physical",
    "reflect/records",
    "reflect/scalars",
    "reflect/scalars2",
    "reflect/walk",
    "psl/psl_clock",
    "psl/psl_pragma",
    "psl/psl_subset",
    "psl/psl_hdl_exprs",
    "psl/psl_sequences",
    "psl/psl_sequences_ops",
    "psl/psl_named",
    "psl/psl_named_formals",
    "psl/psl_temporal",
    "psl/psl_temporal_ext",
    "generics/anonymous_ports_2019_hand",
    "report/assert_user_message",
    "report/concurrent_assert",
    "report/error_then_end",
    "report/failure_exit",
    "report/message_forms",
    "resolution/arch_resolution",
    "resolution/array_record_whole",
    "resolution/array_whole",
    "resolution/bus_guarded",
    "resolution/guarded_bus_port",
    "resolution/custom_resolution",
    "resolution/nested_array_whole",
    "resolution/nested_record_whole",
    "resolution/record_whole",
    "resolution/resolved_array_bounds",
    "resolution/resolved_memo",
    "resolution/resolved_record_port",
    "resolution/shared_var_in_resolver",
    "resolution/std_logic_drivers",
    "resolution/std_logic_table",
    "resolution/user_logic_record",
    "resolution/user_logic_scalar",
    "resolution/user_logic_vector",
    "resolution/whole_std_logic_field",
    "run/agg_targets",
    "run/case_forms",
    "run/cat_null",
    "run/cond_2019",
    "run/driving",
    "run/expr_forms",
    "run/generic_subprog",
    "run/open_actual",
    "run/out_slice",
    "run/record_one_field",
    "run/record_signals",
    "run/shared_generate",
    "run/sig_arrays",
    "run/sig_attrs",
    "run/sig_out",
    "run/sig_params",
    "scalar/array_attrs",
    "scalar/enum_attrs",
    "scalar/int_attrs",
    "scalar/range_attr_attrs",
    "scalar/min_literal",
    "scalar/min_literal_2019",
    "scalar/real_attrs",
    "scalar/real_image",
    "scalar/string_attrs",
    "scalar/subtype_range",
    "scalar/subtype_range_compat",
    "scalar/now_in_bounds",
    "scalar/integer_literal_outside",
    "scalar/time_attrs",
    "scalar/time_bounds",
    "shared/access_vars",
    "shared/pkg_vars",
    "shared/region_vars",
    "shared/elab_call",
    "std93/numeric_std_93",
    "std93/predefined_names",
    "std93/shared_plain",
    "std93/slv_overloads",
    "std93/synopsys_arith_93",
    "std93/textio_93",
    "sigassign/assign_inertial",
    "sigassign/assign_reject",
    "sigassign/assign_transport",
    "sigassign/concurrent_aggregate_choices",
    "sigassign/guarded_explicit_guard",
    "sigassign/guarded_unguarded_target",
    "sigassign/implicit_in_process",
    "sigassign/implicit_in_subprogram",
    "sigassign/last_value_whole",
    "sigassign/logic_gates",
    "sigassign/mux_gates",
    "sigassign/null_waveform",
    "sigassign/null_waveform_mixed",
    "sigassign/out_param_elements",
    "sigassign/plain_copies",
    "sigassign/port_elem_active",
    "sigassign/proc_local_drivers",
    "sigassign/prog_exprs",
    "sigassign/pulse_swallow",
    "sigassign/signal_alias",
    "sigassign/signal_alias_subprogram",
    "sigassign/stable_quiet",
    "sigassign/transaction_init",
    "sigassign/vector_forms",
    "sigassign/waveforms",
    "textio/file_in_architecture",
    "textio/file_in_package",
    "textio/file_roundtrip",
    "textio/std_logic_textio",
    "textio/text_write_raw",
    "textio/write_read_line",
];

const GROUPS: &[&str] = &[
    "scalar",
    "arith",
    "process",
    "report",
    "delta",
    "sigassign",
    "postponed",
    "env",
    "generics",
    "hierarchy",
    "composite",
    "access",
    "protected",
    "shared",
    "std93",
    "resolution",
    "force",
    "textio",
    "run",
    "reflect",
    "ieee",
    "psl",
];

fn corpus() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus"))
}

fn header(src: &str) -> (String, LangRev, Time, bool) {
    let line = src
        .lines()
        .take(15)
        .find_map(|l| l.trim().strip_prefix("-- difftest:"))
        .expect("difftest header");
    let mut top = String::new();
    let mut rev = LangRev::Vhdl2008;
    let mut stop = Time::MAX;
    let relaxed = line.split_whitespace().any(|kv| kv == "relaxed");
    for kv in line.split_whitespace() {
        match kv.split_once('=') {
            Some(("top", v)) => top = v.to_string(),
            Some(("std", "1993")) => rev = LangRev::Vhdl1993,
            Some(("std", "2019")) => rev = LangRev::Vhdl2019,
            Some(("stop", v)) => {
                let ns = v.strip_suffix("ns").expect("stop in ns").parse().unwrap();
                stop = Time::ns(ns);
            }
            _ => {}
        }
    }
    (top, rev, stop, relaxed)
}

/// (severity, time in fs, delta, process index, message).
type Entry = (Severity, u64, u32, usize, String);

struct Reports(Arc<Mutex<Vec<Entry>>>, Arc<Mutex<Vec<u8>>>);

impl Observer for Reports {
    fn report(&mut self, hook: &mut Hook<'_>, r: &Report) {
        self.0.lock().unwrap().push((
            r.severity,
            hook.now().as_fs(),
            hook.delta(),
            r.process.map_or(usize::MAX, |p| p.0 as usize),
            r.message.clone(),
        ));
    }

    fn output(&mut self, _hook: &mut Hook<'_>, _p: yavs_kernel::ProcId, bytes: &[u8]) {
        self.1.lock().unwrap().extend_from_slice(bytes);
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Analyze, elaborate (yavs-elab, which lowers through this crate) and run `file`; its
/// canonical output, or the rendered diagnostics.
fn simulate(file: &Path) -> Result<String, String> {
    let src = std::fs::read_to_string(file).unwrap();
    let (top, rev, stop, relaxed) = header(&src);
    // The checkout's own lib/ tree, not `default_stdlib()` (#7).
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(rev, lib);
    opts.relaxed = relaxed;
    opts.psl = src
        .lines()
        .take(15)
        .any(|l| l.contains("difftest:") && l.split_whitespace().any(|kv| kv == "psl"));
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    let r = db.analyze_file("work", file, rev);
    let text = |db: &Db, ds: &[yavs_diag::Diagnostic]| -> String {
        ds.iter()
            .map(|d| render(&db.sources, d, MessageFormat::Short, false))
            .collect()
    };
    if r.is_err() || db.error_count() > 0 {
        return Err(format!("analysis: {}", text(&db, db.diagnostics())));
    }
    let mut e = yavs_elab::find_top(&mut db, "work", &top)
        .and_then(|t| yavs_elab::elaborate(&mut db, t, &[], &yavs_elab::ElabConfig::default()))
        .map_err(|d| text(&db, &d))?;
    e.lower(&mut db).map_err(|d| text(&db, &d))?;
    let names: Vec<String> = e
        .procs
        .iter()
        .map(|p| {
            let n = e.hier.path_name(p.scope, yavs_elab::Case::Lrm);
            n.strip_suffix(':').unwrap_or(&n).to_string()
        })
        .collect();
    let (mut sim, rt) = e
        .build_runtime(Config::default())
        .map_err(|e| e.to_string())?;
    let reps = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::new(Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Reports(reps.clone(), output.clone())));
    let status = sim.run_until(stop);
    let mut out = String::new();
    // The exit status follows the VHDL assert counts, as `yavs run` (#191).
    let mut code = i32::from(rt.is_some_and(|rt| {
        rt.assert_count(Severity::Error) + rt.assert_count(Severity::Failure) > 0
    }));
    for (sev, t, d, p, m) in reps.lock().unwrap().iter() {
        let s = match sev {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Failure => "failure",
        };
        writeln!(
            out,
            "diag\t{s}\t{t}\t{d}\t{}\t{}",
            names.get(*p).map_or("-", String::as_str),
            escape(m)
        )
        .unwrap();
    }
    match status {
        Ok(RunStatus::Ended(r @ (EndReason::Stop(c) | EndReason::Finish(c)))) => {
            code = c;
            let k = if matches!(r, EndReason::Stop(_)) {
                "stop"
            } else {
                "finish"
            };
            let (t, d) = (sim.now().as_fs(), sim.delta());
            writeln!(out, "diag\t{k}\t{t}\t{d}\t-\t{c}").unwrap();
        }
        Ok(_) => {}
        Err(e) => {
            code = 1;
            let msg = match &e.kind {
                yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
                k => k.to_string(),
            };
            // The reference names the process only for an error inside a subprogram (#506).
            let path = match (&e.process, e.trace.len() > 1) {
                (Some((_, p)), true) => p.as_str(),
                _ => "-",
            };
            writeln!(
                out,
                "diag\tfatal\t{}\t{}\t{path}\t{}",
                e.time.as_fs(),
                e.delta,
                escape(&msg)
            )
            .unwrap();
        }
    }
    let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    for line in text.lines() {
        writeln!(out, "out\t{line}").unwrap();
    }
    Ok(format!("exit\t{code}\n{out}"))
}

/// Replaces the process path of every `diag` event with `-`.
fn path_mask(text: &str) -> String {
    let mut out = String::new();
    for l in text.lines() {
        let mut f: Vec<&str> = l.split('\t').collect();
        if f.len() == 6 && f[0] == "diag" {
            f[4] = "-";
        }
        out.push_str(&f.join("\t"));
        out.push('\n');
    }
    out
}

/// Replaces the text of every `fatal` `diag` event with `-`.
fn fatalmsg_mask(text: &str) -> String {
    let mut out = String::new();
    for l in text.lines() {
        let mut f: Vec<&str> = l.split('\t').collect();
        if f.len() == 6 && f[0] == "diag" && f[1] == "fatal" {
            f[5] = "-";
        }
        out.push_str(&f.join("\t"));
        out.push('\n');
    }
    out
}

/// Sorts each run of `diag` events of one time and delta stably by process path, or by text
/// when the path is ignored too (`by_text`, run after `path_mask`).
fn order_mask(text: &str, by_text: bool) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let key = |l: &str| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() == 6 && f[0] == "diag").then(|| (f[2].to_string(), f[3].to_string()))
    };
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        let k = key(lines[i]);
        let mut j = i + 1;
        while k.is_some() && j < lines.len() && key(lines[j]) == k {
            j += 1;
        }
        let mut run = lines[i..j].to_vec();
        run.sort_by_key(|l| l.split('\t').nth(if by_text { 5 } else { 4 }).unwrap_or(""));
        for l in run {
            out.push_str(l);
            out.push('\n');
        }
        i = j;
    }
    out
}

#[test]
fn corpus_groups() {
    let mut summary = String::new();
    let mut failed = Vec::new();
    for g in GROUPS {
        let dir = match *g {
            // This crate's own runnable tests (`.expected` blessed from sim1 with
            // `tools/difftest/difftest.py --corpus crates/yavs-lower/tests/vhdl/run --bless`).
            "run" => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vhdl/run")),
            g => corpus().join(g),
        };
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
            .collect();
        files.sort();
        for f in files {
            let name = format!("{g}/{}", f.file_stem().unwrap().to_string_lossy());
            let expected = std::fs::read_to_string(f.with_extension("expected")).unwrap();
            // Testbenches write files relative to the working directory: give each its own.
            let cwd = tempfile::tempdir().unwrap();
            std::env::set_current_dir(cwd.path()).unwrap();
            let mut got = simulate(&f);
            // `ignore=order` in the header: same-cycle events of different processes are
            // compared per process, as `tools/difftest` does.
            let mut expected = expected;
            let src = std::fs::read_to_string(&f).unwrap();
            let header = src.lines().take(15).find(|l| l.contains("difftest:"));
            let ignores = |field: &str| {
                header.is_some_and(|h| {
                    h.split_whitespace()
                        .filter_map(|w| w.strip_prefix("ignore="))
                        .any(|v| v.split(',').any(|f| f == field))
                })
            };
            // `ignore=path`: the process path of `diag` events is not compared.
            if ignores("path") {
                expected = path_mask(&expected);
                got = got.map(|o| path_mask(&o));
            }
            if ignores("order") {
                let by_text = ignores("path");
                expected = order_mask(&expected, by_text);
                got = got.map(|o| order_mask(&o, by_text));
            }
            // `ignore=fatalmsg`: the text of a fatal run-time error is not compared.
            if ignores("fatalmsg") {
                expected = fatalmsg_mask(&expected);
                got = got.map(|o| fatalmsg_mask(&o));
            }
            let pass = matches!(&got, Ok(o) if o == &expected);
            let status = match &got {
                _ if pass => "PASS".to_string(),
                Ok(o) => format!("DIFF\n--- expected\n{expected}--- got\n{o}"),
                Err(e) => format!("FAIL {e}"),
            };
            writeln!(summary, "{name}: {status}").unwrap();
            if !pass && PASS.contains(&name.as_str()) {
                failed.push(name);
            }
        }
    }
    println!("{summary}");
    assert!(failed.is_empty(), "regressed: {failed:?}\n{summary}");
}

/// Every subprogram of a package body lowers and verifies; each instruction has a statement
/// with a source span (the verifier checks the ids, this checks the spans).
#[test]
fn package_subprograms() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_scalar.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(
        &db,
        LowerOptions {
            coverage: true,
            ..LowerOptions::default()
        },
    );
    let funcs = l.subprograms_of(body).unwrap();
    assert_eq!(funcs.len(), 5);
    let p = l.finish().unwrap().program;
    for (_, f) in p.funcs.iter() {
        for s in &f.loc {
            let info = p.stmts.get(*s).unwrap();
            assert!(info.span.end > info.span.start, "{}: empty span", f.name);
        }
    }
    // `noisy` waits, so it may suspend; `fib` is pure and recursive.
    let flags = |n: &str| p.funcs.values().iter().find(|f| f.name == n).unwrap().flags;
    assert!(flags("noisy").contains(yavs_ir::FuncFlags::MAY_SUSPEND));
    assert!(flags("fib").contains(yavs_ir::FuncFlags::PURE));
    assert!(!p.cov.is_empty());
    // Without the option there are no counters at all (zero cost, #131).
    let mut l = Lowerer::new(&db, LowerOptions::default());
    l.subprograms_of(body).unwrap();
    assert!(l.finish().unwrap().program.cov.is_empty());
}

/// The bounds of a statically sized array variable are a constant: indexing it reads no
/// descriptor from the frame (every `Pos` reads a constant-pool descriptor).
#[test]
fn static_array_bounds_are_constant() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_static_array.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let funcs = l.subprograms_of(body).unwrap();
    assert_eq!(funcs.len(), 1);
    let p = l.finish().unwrap().program;
    let f = p.func(funcs[0]).unwrap();
    let konst: Vec<_> = f
        .code
        .iter()
        .filter_map(|i| match *i {
            yavs_ir::Inst::AddrOf {
                dst,
                space: yavs_ir::AddrSpace::Const(_),
                ..
            } => Some(dst),
            _ => None,
        })
        .collect();
    let pos: Vec<_> = f
        .code
        .iter()
        .filter_map(|i| match *i {
            yavs_ir::Inst::Pos { dope, .. } => Some(dope),
            _ => None,
        })
        .collect();
    assert!(!pos.is_empty());
    for d in pos {
        assert!(konst.contains(&d), "{}", p.dump_func(funcs[0]));
    }
}

/// The inner loop of a matrix built from an array of vectors computes only what changes per
/// element: the position of `i`, the element descriptor, the lengths and their products are
/// computed before it (#526).
#[test]
fn loop_invariant_code_leaves_the_inner_loop() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_loop_invariant.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let funcs = l.subprograms_of(body).unwrap();
    assert_eq!(funcs.len(), 1);
    let p = l.finish().unwrap().program;
    let f = p.func(funcs[0]).unwrap();
    // The block of the element write: before, four positions and four lengths.
    let blk = f
        .blocks
        .values()
        .iter()
        .find(|b| {
            f.code[b.start as usize..b.end as usize]
                .iter()
                .any(|i| matches!(i, yavs_ir::Inst::LSet { .. }))
        })
        .unwrap();
    let code = &f.code[blk.start as usize..blk.end as usize];
    let count = |m: fn(&yavs_ir::Inst) -> bool| code.iter().filter(|i| m(i)).count();
    let dump = p.dump_func(funcs[0]);
    assert_eq!(
        count(|i| matches!(i, yavs_ir::Inst::DopeLen { .. })),
        0,
        "{dump}"
    );
    assert!(
        count(|i| matches!(i, yavs_ir::Inst::Pos { .. })) <= 2,
        "{dump}"
    );
}

/// A periodic composite constant (an array of records holding long logic vectors and arrays of
/// one record) is a compact pool constant, not its written-out bytes (#446).
#[test]
fn periodic_constant_is_compact() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_periodic_const.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().to_path_buf();
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let funcs = l.subprograms_of(body).unwrap();
    let p = l.finish().unwrap().program;
    let big = p
        .consts
        .values()
        .iter()
        .filter(|c| matches!(c, yavs_ir::ConstData::Bytes(b) if b.len() > 4096))
        .count();
    assert_eq!(big, 0, "written-out bytes in the pool");
    let compact = p
        .consts
        .values()
        .iter()
        .filter(|c| {
            matches!(
                c,
                yavs_ir::ConstData::Repeat { .. } | yavs_ir::ConstData::Runs(_)
            )
        })
        .count();
    assert!(compact > 0, "{}", p.dump_func(funcs[0]));
}

/// A function that folds a logic vector with the identity of one gate is called as the native
/// of the unary operator; a look-alike with another start value stays a call (#531).
#[test]
fn reduction_idiom_calls_the_native() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_reduce.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let tmp = tempfile::tempdir().expect("tmp");
    opts.libraries
        .add_spec(&format!("work:{}", tmp.path().display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let funcs = l.subprograms_of(body).unwrap();
    let p = l.finish().unwrap().program;
    let wrap = funcs
        .iter()
        .copied()
        .find(|&f| p.func(f).unwrap().name.contains("wrap"))
        .unwrap();
    let code = &p.func(wrap).unwrap().code;
    let reduce = "yavs.ieee.std_logic_1164.\"or\"(std_ulogic_vector)std_ulogic";
    let natives = code
        .iter()
        .filter(|i| match **i {
            yavs_ir::Inst::CallNative { native, .. } => {
                p.natives.get(native).is_some_and(|n| n.symbol == reduce)
            }
            _ => false,
        })
        .count();
    // `any_f` (start value assigned) and `any_d` (start value as the default) are the native;
    // `all_f`, which has no gate in its loop, is not.
    assert_eq!(natives, 2, "{}", p.dump_func(wrap));
}
