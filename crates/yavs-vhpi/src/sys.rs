//! Rust declarations of the VHPI C interface (IEEE 1076-2019, Annex B).
//!
//! Names follow the C header (`include/vhpi_user.h`) one to one, so code
//! reads the same on both sides of the ABI. C enums are `c_int` type
//! aliases with one constant per enumerator. The constant tables
//! (`*_VALUES`) exist so tests can check every value against the C
//! compiler.

#![allow(non_camel_case_types, non_upper_case_globals, non_snake_case)]

use std::ffi::{c_char, c_int, c_void};

/// Declares C constants of one type plus a `(name, value)` table of them.
macro_rules! c_consts {
    ($table:ident: $ty:ty { $($name:ident = $val:expr,)* }) => {
        $(pub const $name: $ty = $val;)*
        /// Every constant of this group, for ABI tests.
        pub const $table: &[(&str, i64)] = &[$((stringify!($name), $name as i64)),*];
    };
}

/// Opaque object handle (`uint32_t *` in C). yavs encodes a generational
/// arena id in the pointer value; it is never dereferenced.
pub type vhpiHandleT = *mut u32;
pub type vhpiEnumT = u32;
pub type vhpiSmallEnumT = u8;
pub type vhpiIntT = i32;
pub type vhpiLongIntT = i64;
pub type vhpiCharT = u8;
pub type vhpiRealT = f64;
pub type vhpiSmallPhysT = i32;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct vhpiPhysT {
    pub high: i32,
    pub low: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct vhpiTimeT {
    pub high: i32,
    pub low: u32,
}

pub type vhpiFormatT = c_int;
pub type vhpiClassKindT = c_int;
pub type vhpiOneToOneT = c_int;
pub type vhpiOneToManyT = c_int;
pub type vhpiIntPropertyT = c_int;
pub type vhpiStrPropertyT = c_int;
pub type vhpiRealPropertyT = c_int;
pub type vhpiPhysPropertyT = c_int;
pub type vhpiCapabibilityT = c_int;
pub type vhpiOpenModeT = c_int;
pub type vhpiModeT = c_int;
pub type vhpiSigKindT = c_int;
pub type vhpiStaticnessT = c_int;
pub type vhpiPredefAttrT = c_int;
pub type vhpiAttrKindT = c_int;
pub type vhpiEntityClassT = c_int;
pub type vhpiAccessT = c_int;
pub type vhpiStateT = c_int;
pub type vhpiCompInstKindT = c_int;
pub type vhpiPhaseT = c_int;
pub type vhpiSeverityT = c_int;
pub type vhpiAutomaticRestoreT = c_int;
pub type vhpiPutValueModeT = c_int;
pub type vhpiDelayModeT = c_int;
pub type vhpiSimControlT = c_int;
pub type vhpiForeignKindT = c_int;

#[repr(C)]
#[derive(Clone, Copy)]
pub union vhpiValueU {
    pub enumv: vhpiEnumT,
    pub enumvs: *mut vhpiEnumT,
    pub smallenumv: vhpiSmallEnumT,
    pub smallenumvs: *mut vhpiSmallEnumT,
    pub intg: vhpiIntT,
    pub intgs: *mut vhpiIntT,
    pub longintg: vhpiLongIntT,
    pub longintgs: *mut vhpiLongIntT,
    pub real: vhpiRealT,
    pub reals: *mut vhpiRealT,
    pub smallphys: vhpiSmallPhysT,
    pub smallphyss: *mut vhpiSmallPhysT,
    pub phys: vhpiPhysT,
    pub physs: *mut vhpiPhysT,
    pub time: vhpiTimeT,
    pub times: *mut vhpiTimeT,
    pub ch: vhpiCharT,
    pub str_: *mut vhpiCharT,
    pub ptr: *mut c_void,
    pub ptrs: *mut *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct vhpiValueT {
    pub format: vhpiFormatT,
    pub bufSize: usize,
    pub numElems: i32,
    pub unit: vhpiPhysT,
    pub value: vhpiValueU,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct vhpiErrorInfoT {
    pub severity: vhpiSeverityT,
    pub message: *mut c_char,
    pub str_: *mut c_char,
    pub file: *mut c_char,
    pub line: i32,
}

pub type vhpiCbRoutineT = Option<unsafe extern "C" fn(*const vhpiCbDataT)>;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct vhpiCbDataT {
    pub reason: i32,
    pub cb_rtn: vhpiCbRoutineT,
    pub obj: vhpiHandleT,
    pub time: *mut vhpiTimeT,
    pub value: *mut vhpiValueT,
    pub user_data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct vhpiForeignDataT {
    pub kind: vhpiForeignKindT,
    pub libraryName: *mut c_char,
    pub modelName: *mut c_char,
    pub elabf: vhpiCbRoutineT,
    pub execf: vhpiCbRoutineT,
}

pub type vhpiUserFctT = Option<unsafe extern "C" fn() -> c_int>;
pub type vhpiRegistrationFctT = Option<unsafe extern "C" fn()>;

c_consts! { MISC_VALUES: i32 {
    vhpiUndefined = -1,
    vhpiNoActivity = -1,
    vhpiSensitivitys = DEPRECATED_vhpiSensitivitys,
} }

c_consts! { VHPIFORMATT_VALUES: vhpiFormatT {
    vhpiBinStrVal = 1,
    vhpiOctStrVal = 2,
    vhpiDecStrVal = 3,
    vhpiHexStrVal = 4,
    vhpiEnumVal = 5,
    vhpiIntVal = 6,
    vhpiLogicVal = 7,
    vhpiRealVal = 8,
    vhpiStrVal = 9,
    vhpiCharVal = 10,
    vhpiTimeVal = 11,
    vhpiPhysVal = 12,
    vhpiObjTypeVal = 13,
    vhpiPtrVal = 14,
    vhpiEnumVecVal = 15,
    vhpiIntVecVal = 16,
    vhpiLogicVecVal = 17,
    vhpiRealVecVal = 18,
    vhpiTimeVecVal = 19,
    vhpiPhysVecVal = 20,
    vhpiPtrVecVal = 21,
    vhpiRawDataVal = 22,
    vhpiSmallEnumVal = 23,
    vhpiSmallEnumVecVal = 24,
    vhpiLongIntVal = 25,
    vhpiLongIntVecVal = 26,
    vhpiSmallPhysVal = 27,
    vhpiSmallPhysVecVal = 28,
} }

c_consts! { VHPICLASSKINDT_VALUES: vhpiClassKindT {
    vhpiAccessTypeDeclK = 1001,
    vhpiAggregateK = 1002,
    vhpiAliasDeclK = 1003,
    vhpiAllK = 1004,
    vhpiAllocatorK = 1005,
    vhpiAnyCollectionK = 1006,
    vhpiArchBodyK = 1007,
    vhpiArgvK = 1008,
    vhpiArrayTypeDeclK = 1009,
    DEPRECATED_vhpiAssertStmtK = 1010,
    vhpiAssocElemK = 1011,
    vhpiAttrDeclK = 1012,
    vhpiAttrSpecK = 1013,
    DEPRECATED_vhpiBinaryExprK = 1014,
    vhpiBitStringLiteralK = 1015,
    vhpiBlockConfigK = 1016,
    vhpiBlockStmtK = 1017,
    vhpiBranchK = 1018,
    vhpiCallbackK = 1019,
    vhpiCaseStmtK = 1020,
    vhpiCharLiteralK = 1021,
    vhpiCompConfigK = 1022,
    vhpiCompDeclK = 1023,
    vhpiCompInstStmtK = 1024,
    vhpiCondSigAssignStmtK = 1025,
    vhpiCondWaveformK = 1026,
    vhpiConfigDeclK = 1027,
    vhpiConstDeclK = 1028,
    vhpiConstParamDeclK = 1029,
    vhpiConvFuncK = 1030,
    vhpiDerefObjK = 1031,
    vhpiDisconnectSpecK = 1032,
    vhpiDriverK = 1033,
    vhpiDriverCollectionK = 1034,
    vhpiElemAssocK = 1035,
    vhpiElemDeclK = 1036,
    vhpiEntityClassEntryK = 1037,
    vhpiEntityDeclK = 1038,
    vhpiEnumLiteralK = 1039,
    vhpiEnumRangeK = 1040,
    vhpiEnumTypeDeclK = 1041,
    vhpiExitStmtK = 1042,
    vhpiFileDeclK = 1043,
    vhpiFileParamDeclK = 1044,
    vhpiFileTypeDeclK = 1045,
    vhpiFloatRangeK = 1046,
    vhpiFloatTypeDeclK = 1047,
    vhpiForGenerateK = 1048,
    vhpiForLoopK = 1049,
    vhpiForeignfK = 1050,
    vhpiFuncCallK = 1051,
    vhpiFuncDeclK = 1052,
    vhpiGenericDeclK = 1053,
    vhpiGroupDeclK = 1054,
    vhpiGroupTempDeclK = 1055,
    vhpiIfGenerateK = 1056,
    vhpiIfStmtK = 1057,
    vhpiInPortK = 1058,
    vhpiIndexedNameK = 1059,
    vhpiIntLiteralK = 1060,
    vhpiIntRangeK = 1061,
    vhpiIntTypeDeclK = 1062,
    vhpiIteratorK = 1063,
    vhpiLibraryDeclK = 1064,
    DEPRECATED_vhpiLoopStmtK = 1065,
    vhpiNextStmtK = 1066,
    vhpiNullLiteralK = 1067,
    vhpiNullStmtK = 1068,
    DEPRECATED_vhpiOperatorK = 1069,
    vhpiOthersK = 1070,
    vhpiOutPortK = 1071,
    vhpiPackBodyK = 1072,
    vhpiPackDeclK = 1073,
    vhpiPackInstK = 1074,
    vhpiParamAttrNameK = 1075,
    vhpiPhysLiteralK = 1076,
    vhpiPhysRangeK = 1077,
    vhpiPhysTypeDeclK = 1078,
    vhpiPortDeclK = 1079,
    DEPRECATED_vhpiProcCallStmtK = 1080,
    vhpiProcDeclK = 1081,
    vhpiProcessStmtK = 1082,
    DEPRECATED_vhpiProtectedTypeK = 1083,
    vhpiProtectedTypeBodyK = 1084,
    vhpiProtectedTypeDeclK = 1085,
    vhpiRealLiteralK = 1086,
    vhpiRecordTypeDeclK = 1087,
    vhpiReportStmtK = 1088,
    vhpiReturnStmtK = 1089,
    vhpiRootInstK = 1090,
    vhpiSelectSigAssignStmtK = 1091,
    vhpiSelectWaveformK = 1092,
    vhpiSelectedNameK = 1093,
    vhpiSigDeclK = 1094,
    vhpiSigParamDeclK = 1095,
    vhpiSimpAttrNameK = 1096,
    vhpiSimpleSigAssignStmtK = 1097,
    vhpiSliceNameK = 1098,
    vhpiStringLiteralK = 1099,
    vhpiSubpBodyK = 1100,
    vhpiSubtypeDeclK = 1101,
    DEPRECATED_vhpiSubtypeIndicK = 1102,
    vhpiToolK = 1103,
    vhpiTransactionK = 1104,
    vhpiTypeConvK = 1105,
    DEPRECATED_vhpiUnaryExprK = 1106,
    vhpiUnitDeclK = 1107,
    vhpiUserAttrNameK = 1108,
    vhpiVarAssignStmtK = 1109,
    vhpiVarDeclK = 1110,
    vhpiVarParamDeclK = 1111,
    vhpiWaitStmtK = 1112,
    vhpiWaveformElemK = 1113,
    vhpiWhileLoopK = 1114,
    vhpiQualifiedExprK = 1115,
    vhpiUseClauseK = 1116,
    vhpiConcAssertStmtK = 1117,
    vhpiConcProcCallStmtK = 1118,
    vhpiForeverLoopK = 1119,
    vhpiSeqAssertStmtK = 1120,
    vhpiSeqProcCallStmtK = 1121,
    vhpiSeqSigAssignStmtK = 1122,
    vhpiProtectedTypeInstK = 1123,
} }

c_consts! { VHPIONETOONET_VALUES: vhpiOneToOneT {
    vhpiAbstractLiteral = 1301,
    vhpiActual = 1302,
    vhpiAll = 1303,
    vhpiAttrDecl = 1304,
    vhpiAttrSpec = 1305,
    vhpiBaseType = 1306,
    vhpiBaseUnit = 1307,
    DEPRECATED_vhpiBasicSignal = 1308,
    vhpiBlockConfig = 1309,
    vhpiCaseExpr = 1310,
    vhpiCondExpr = 1311,
    vhpiConfigDecl = 1312,
    vhpiConfigSpec = 1313,
    vhpiConstraint = 1314,
    vhpiContributor = 1315,
    vhpiCurCallback = 1316,
    DEPRECATED_vhpiCurEqProcess = 1317,
    vhpiCurStackFrame = 1318,
    vhpiDerefObj = 1319,
    DEPRECATED_vhpiDecl = 1320,
    vhpiDesignUnit = 1321,
    vhpiDownStack = 1322,
    DEPRECATED_vhpiElemSubtype = 1323,
    vhpiEntityAspect = 1324,
    vhpiEntityDecl = 1325,
    vhpiEqProcessStmt = 1326,
    vhpiExpr = 1327,
    vhpiFormal = 1328,
    vhpiFuncDecl = 1329,
    vhpiGroupTempDecl = 1330,
    vhpiGuardExpr = 1331,
    vhpiGuardSig = 1332,
    vhpiImmRegion = 1333,
    vhpiInPort = 1334,
    vhpiInitExpr = 1335,
    DEPRECATED_vhpiIterScheme = 1336,
    vhpiLeftExpr = 1337,
    vhpiLexicalScope = 1338,
    vhpiLhsExpr = 1339,
    vhpiLocal = 1340,
    vhpiLogicalExpr = 1341,
    DEPRECATED_vhpiName = 1342,
    DEPRECATED_vhpiOperator = 1343,
    vhpiOthers = 1344,
    vhpiOutPort = 1345,
    vhpiParamDecl = 1346,
    DEPRECATED_vhpiParamExpr = 1347,
    vhpiParent = 1348,
    vhpiPhysLiteral = 1349,
    vhpiPrefix = 1350,
    vhpiPrimaryUnit = 1351,
    vhpiProtectedTypeBody = 1352,
    vhpiProtectedTypeDecl = 1353,
    vhpiRejectTime = 1354,
    vhpiReportExpr = 1355,
    vhpiResolFunc = 1356,
    vhpiReturnExpr = 1357,
    DEPRECATED_vhpiReturnTypeMark = 1358,
    vhpiRhsExpr = 1359,
    vhpiRightExpr = 1360,
    vhpiRootInst = 1361,
    vhpiSelectExpr = 1362,
    vhpiSeverityExpr = 1363,
    vhpiSimpleName = 1364,
    vhpiSubpBody = 1365,
    vhpiSubpDecl = 1366,
    DEPRECATED_vhpiSubtype = 1367,
    vhpiSuffix = 1368,
    vhpiTimeExpr = 1369,
    vhpiTimeOutExpr = 1370,
    vhpiTool = 1371,
    vhpiType = 1372,
    DEPRECATED_vhpiTypeMark = 1373,
    vhpiUnitDecl = 1374,
    vhpiUpStack = 1375,
    vhpiUpperRegion = 1376,
    vhpiUse = 1377,
    vhpiValExpr = 1378,
    DEPRECATED_vhpiValSubtype = 1379,
    vhpiElemType = 1380,
    vhpiFirstNamedType = 1381,
    vhpiReturnType = 1382,
    vhpiValType = 1383,
    vhpiCurRegion = 1384,
    vhpiSignal = 1385,
    vhpiLibraryDecl = 1386,
    vhpiSimNet = 1387,
    vhpiAliasedName = 1388,
    vhpiCompDecl = 1389,
    vhpiProtectedTypeInst = 1390,
    vhpiGenIndex = 1391,
} }

c_consts! { VHPIONETOMANYT_VALUES: vhpiOneToManyT {
    vhpiAliasDecls = 1501,
    vhpiArgvs = 1502,
    vhpiAttrDecls = 1503,
    vhpiAttrSpecs = 1504,
    vhpiBasicSignals = 1505,
    vhpiBlockStmts = 1506,
    vhpiBranchs = 1507,
    vhpiChoices = 1509,
    vhpiCompInstStmts = 1510,
    DEPRECATED_vhpiCondExprs = 1511,
    vhpiCondWaveforms = 1512,
    vhpiConfigItems = 1513,
    vhpiConfigSpecs = 1514,
    vhpiConstDecls = 1515,
    vhpiConstraints = 1516,
    DEPRECATED_vhpiContributors = 1517,
    vhpiDecls = 1519,
    vhpiDepUnits = 1520,
    vhpiDesignUnits = 1521,
    vhpiDrivenSigs = 1522,
    vhpiDrivers = 1523,
    vhpiElemAssocs = 1524,
    DEPRECATED_vhpiEntityClassEntrys = 1525,
    vhpiEntityDesignators = 1526,
    vhpiEnumLiterals = 1527,
    vhpiForeignfs = 1528,
    vhpiGenericAssocs = 1529,
    vhpiGenericDecls = 1530,
    vhpiIndexExprs = 1531,
    vhpiIndexedNames = 1532,
    vhpiInternalRegions = 1533,
    vhpiMembers = 1534,
    vhpiPackInsts = 1535,
    vhpiParamAssocs = 1536,
    vhpiParamDecls = 1537,
    vhpiPortAssocs = 1538,
    vhpiPortDecls = 1539,
    vhpiRecordElems = 1540,
    vhpiSelectWaveforms = 1541,
    vhpiSelectedNames = 1542,
    DEPRECATED_vhpiSensitivitys = 1543,
    vhpiSeqStmts = 1544,
    vhpiSigAttrs = 1545,
    vhpiSigDecls = 1546,
    vhpiSigNames = 1547,
    vhpiSignals = 1548,
    DEPRECATED_vhpiSpecNames = 1549,
    vhpiSpecs = 1550,
    vhpiStmts = 1551,
    vhpiTransactions = 1552,
    DEPRECATED_vhpiTypeMarks = 1553,
    vhpiUnitDecls = 1554,
    vhpiUses = 1555,
    vhpiVarDecls = 1556,
    vhpiWaveformElems = 1557,
    vhpiLibraryDecls = 1558,
    vhpiLocalLoads = 1559,
    vhpiOptimizedLoads = 1560,
    vhpiTypes = 1561,
    vhpiUseClauses = 1562,
    vhpiGenerateStmts = 1563,
    vhpiLocalContributors = 1564,
    vhpiOptimizedContributors = 1565,
    vhpiParamExprs = 1566,
    vhpiEqProcessStmts = 1567,
    vhpiEntityClassEntries = 1568,
    vhpiSensitivities = 1569,
} }

c_consts! { VHPIINTPROPERTYT_VALUES: vhpiIntPropertyT {
    vhpiAccessP = 1001,
    vhpiArgcP = 1002,
    vhpiAttrKindP = 1003,
    vhpiBaseIndexP = 1004,
    vhpiBeginLineNoP = 1005,
    vhpiEndLineNoP = 1006,
    vhpiEntityClassP = 1007,
    vhpiForeignKindP = 1008,
    vhpiFrameLevelP = 1009,
    vhpiGenerateIndexP = 1010,
    vhpiIntValP = 1011,
    vhpiIsAnonymousP = 1012,
    vhpiIsBasicP = 1013,
    vhpiIsCompositeP = 1014,
    vhpiIsDefaultP = 1015,
    vhpiIsDeferredP = 1016,
    vhpiIsDiscreteP = 1017,
    vhpiIsForcedP = 1018,
    vhpiIsForeignP = 1019,
    vhpiIsGuardedP = 1020,
    vhpiIsImplicitDeclP = 1021,
    DEPRECATED_vhpiIsInvalidP = 1022,
    vhpiIsLocalP = 1023,
    vhpiIsNamedP = 1024,
    vhpiIsNullP = 1025,
    vhpiIsOpenP = 1026,
    vhpiIsPLIP = 1027,
    vhpiIsPassiveP = 1028,
    vhpiIsPostponedP = 1029,
    vhpiIsProtectedTypeP = 1030,
    vhpiIsPureP = 1031,
    vhpiIsResolvedP = 1032,
    vhpiIsScalarP = 1033,
    vhpiIsSeqStmtP = 1034,
    vhpiIsSharedP = 1035,
    vhpiIsTransportP = 1036,
    vhpiIsUnaffectedP = 1037,
    vhpiIsUnconstrainedP = 1038,
    vhpiIsUninstantiatedP = 1039,
    vhpiIsUpP = 1040,
    vhpiIsVitalP = 1041,
    vhpiIteratorTypeP = 1042,
    vhpiKindP = 1043,
    vhpiLeftBoundP = 1044,
    DEPRECATED_vhpiLevelP = 1045,
    vhpiLineNoP = 1046,
    vhpiLineOffsetP = 1047,
    vhpiLoopIndexP = 1048,
    vhpiModeP = 1049,
    vhpiNumDimensionsP = 1050,
    DEPRECATED_vhpiNumFieldsP = 1051,
    vhpiNumGensP = 1052,
    vhpiNumLiteralsP = 1053,
    vhpiNumMembersP = 1054,
    vhpiNumParamsP = 1055,
    vhpiNumPortsP = 1056,
    vhpiOpenModeP = 1057,
    vhpiPhaseP = 1058,
    vhpiPositionP = 1059,
    vhpiPredefAttrP = 1060,
    vhpiReasonP = 1062,
    vhpiRightBoundP = 1063,
    vhpiSigKindP = 1064,
    vhpiSizeP = 1065,
    vhpiStartLineNoP = 1066,
    vhpiStateP = 1067,
    vhpiStaticnessP = 1068,
    vhpiVHDLversionP = 1069,
    vhpiIdP = 1070,
    vhpiCapabilitiesP = 1071,
    vhpiAutomaticRestoreP = 1072,
    vhpiCompInstKindP = 1073,
    vhpiIsBuiltInP = 1074,
    vhpiIsDynamicP = 1075,
    vhpiIsOperatorP = 1076,
    vhpiNumFieldsP = 1077,
} }

c_consts! { VHPISTRPROPERTYT_VALUES: vhpiStrPropertyT {
    vhpiCaseNameP = 1301,
    vhpiCompNameP = 1302,
    vhpiDefNameP = 1303,
    vhpiFileNameP = 1304,
    vhpiFullCaseNameP = 1305,
    vhpiFullNameP = 1306,
    vhpiKindStrP = 1307,
    vhpiLabelNameP = 1308,
    vhpiLibLogicalNameP = 1309,
    vhpiLibPhysicalNameP = 1310,
    vhpiLogicalNameP = 1311,
    vhpiLoopLabelNameP = 1312,
    vhpiNameP = 1313,
    DEPRECATED_vhpiOpNameP = 1314,
    vhpiStrValP = 1315,
    vhpiToolVersionP = 1316,
    vhpiUnitNameP = 1317,
    vhpiSaveRestartLocationP = 1318,
    vhpiCompInstNameP = 1319,
    vhpiInstNamesP = 1320,
    vhpiSignatureNameP = 1321,
    vhpiSpecNameP = 1322,
} }

c_consts! { VHPIREALPROPERTYT_VALUES: vhpiRealPropertyT {
    vhpiFloatLeftBoundP = 1601,
    vhpiFloatRightBoundP = 1602,
    vhpiRealValP = 1603,
} }

c_consts! { VHPIPHYSPROPERTYT_VALUES: vhpiPhysPropertyT {
    vhpiPhysLeftBoundP = 1651,
    vhpiPhysPositionP = 1652,
    vhpiPhysRightBoundP = 1653,
    vhpiPhysValP = 1654,
    DEPRECATED_vhpiPrecisionP = 1655,
    DEPRECATED_vhpiSimTimeUnitP = 1656,
    vhpiResolutionLimitP = 1657,
    vhpiTimeP = 1658,
} }

c_consts! { VHPICAPABIBILITYT_VALUES: vhpiCapabibilityT {
    vhpiProvidesHierarchy = 1,
    vhpiProvidesStaticAccess = 2,
    vhpiProvidesConnectivity = 4,
    vhpiProvidesPostAnalysis = 8,
    vhpiProvidesForeignModel = 16,
    vhpiProvidesAdvancedForeignModel = 32,
    vhpiProvidesSaveRestart = 64,
    vhpiProvidesReset = 128,
    vhpiProvidesDebugRuntime = 256,
    vhpiProvidesAdvancedDebugRuntime = 512,
    vhpiProvidesDynamicElab = 1024,
} }

c_consts! { VHPIOPENMODET_VALUES: vhpiOpenModeT {
    vhpiInOpen = 1001,
    vhpiOutOpen = 1002,
    vhpiReadOpen = 1003,
    vhpiWriteOpen = 1004,
    vhpiAppendOpen = 1005,
} }

c_consts! { VHPIMODET_VALUES: vhpiModeT {
    vhpiInMode = 1001,
    vhpiOutMode = 1002,
    vhpiInoutMode = 1003,
    vhpiBufferMode = 1004,
    vhpiLinkageMode = 1005,
} }

c_consts! { VHPISIGKINDT_VALUES: vhpiSigKindT {
    vhpiRegister = 1001,
    vhpiBus = 1002,
    vhpiNormal = 1003,
} }

c_consts! { VHPISTATICNESST_VALUES: vhpiStaticnessT {
    vhpiLocallyStatic = 1001,
    vhpiGloballyStatic = 1002,
    vhpiDynamic = 1003,
} }

c_consts! { VHPIPREDEFATTRT_VALUES: vhpiPredefAttrT {
    vhpiActivePA = 1001,
    vhpiAscendingPA = 1002,
    vhpiBasePA = 1003,
    vhpiDelayedPA = 1004,
    vhpiDrivingPA = 1005,
    vhpiDriving_valuePA = 1006,
    vhpiEventPA = 1007,
    vhpiHighPA = 1008,
    vhpiImagePA = 1009,
    vhpiInstance_namePA = 1010,
    vhpiLast_activePA = 1011,
    vhpiLast_eventPA = 1012,
    vhpiLast_valuePA = 1013,
    vhpiLeftPA = 1014,
    vhpiLeftofPA = 1015,
    vhpiLengthPA = 1016,
    vhpiLowPA = 1017,
    vhpiPath_namePA = 1018,
    vhpiPosPA = 1019,
    vhpiPredPA = 1020,
    vhpiQuietPA = 1021,
    vhpiRangePA = 1022,
    vhpiReverse_rangePA = 1023,
    vhpiRightPA = 1024,
    vhpiRightofPA = 1025,
    vhpiSimple_namePA = 1026,
    vhpiStablePA = 1027,
    vhpiSuccPA = 1028,
    vhpiTransactionPA = 1029,
    vhpiValPA = 1030,
    vhpiValuePA = 1031,
} }

c_consts! { VHPIATTRKINDT_VALUES: vhpiAttrKindT {
    vhpiFunctionAK = 1,
    vhpiRangeAK = 2,
    vhpiSignalAK = 3,
    vhpiTypeAK = 4,
    vhpiValueAK = 5,
} }

c_consts! { VHPIENTITYCLASST_VALUES: vhpiEntityClassT {
    vhpiEntityEC = 1001,
    vhpiArchitectureEC = 1002,
    vhpiConfigurationEC = 1003,
    vhpiProcedureEC = 1004,
    vhpiFunctionEC = 1005,
    vhpiPackageEC = 1006,
    vhpiTypeEC = 1007,
    vhpiSubtypeEC = 1008,
    vhpiConstantEC = 1009,
    vhpiSignalEC = 1010,
    vhpiVariableEC = 1011,
    vhpiComponentEC = 1012,
    vhpiLabelEC = 1013,
    vhpiLiteralEC = 1014,
    vhpiUnitsEC = 1015,
    vhpiFileEC = 1016,
    vhpiGroupEC = 1017,
} }

c_consts! { VHPIACCESST_VALUES: vhpiAccessT {
    vhpiRead = 1,
    vhpiWrite = 2,
    vhpiConnectivity = 4,
    vhpiNoAccess = 8,
} }

c_consts! { VHPISTATET_VALUES: vhpiStateT {
    vhpiEnable = 0,
    vhpiDisable = 1,
    vhpiMature = 2,
} }

c_consts! { VHPICOMPINSTKINDT_VALUES: vhpiCompInstKindT {
    vhpiDirect = 0,
    vhpiComp = 1,
    vhpiConfig = 2,
} }

c_consts! { VHPIPHASET_VALUES: vhpiPhaseT {
    vhpiRegistrationPhase = 1,
    vhpiAnalysisPhase = 2,
    vhpiElaborationPhase = 3,
    vhpiInitializationPhase = 4,
    vhpiSimulationPhase = 5,
    vhpiTerminationPhase = 6,
    vhpiSavePhase = 7,
    vhpiRestartPhase = 8,
    vhpiResetPhase = 9,
} }

c_consts! { VHPISEVERITYT_VALUES: vhpiSeverityT {
    vhpiNote = 1,
    vhpiWarning = 2,
    vhpiError = 3,
    vhpiFailure = 6,
    vhpiSystem = 4,
    vhpiInternal = 5,
} }

c_consts! { VHPIAUTOMATICRESTORET_VALUES: vhpiAutomaticRestoreT {
    vhpiRestoreAll = 1,
    vhpiRestoreUserData = 2,
    vhpiRestoreHandles = 4,
    vhpiRestoreCallbacks = 8,
} }

c_consts! { VHPIPUTVALUEMODET_VALUES: vhpiPutValueModeT {
    vhpiDeposit = 0,
    vhpiDepositPropagate = 1,
    vhpiForce = 2,
    vhpiForcePropagate = 3,
    vhpiRelease = 4,
    vhpiSizeConstraint = 5,
} }

c_consts! { VHPIDELAYMODET_VALUES: vhpiDelayModeT {
    vhpiInertial = 0,
    vhpiTransport = 1,
} }

c_consts! { VHPISIMCONTROLT_VALUES: vhpiSimControlT {
    vhpiStop = 0,
    vhpiFinish = 1,
    vhpiReset = 2,
} }

c_consts! { VHPIFOREIGNKINDT_VALUES: vhpiForeignKindT {
    vhpiArchF = 1,
    vhpiFuncF = 2,
    vhpiProcF = 3,
    vhpiLibF = 4,
    vhpiAppF = 5,
} }

c_consts! { DEFINE_VALUES: i32 {
    vhpiU = 0,
    vhpiX = 1,
    vhpi0 = 2,
    vhpi1 = 3,
    vhpiZ = 4,
    vhpiW = 5,
    vhpiL = 6,
    vhpiH = 7,
    vhpiDontCare = 8,
    vhpibit0 = 0,
    vhpibit1 = 1,
    vhpiFalse = 0,
    vhpiTrue = 1,
    vhpiCbValueChange = 1001,
    vhpiCbForce = 1002,
    vhpiCbRelease = 1003,
    vhpiCbTransaction = 1004,
    vhpiCbStmt = 1005,
    vhpiCbResume = 1006,
    vhpiCbSuspend = 1007,
    vhpiCbStartOfSubpCall = 1008,
    vhpiCbEndOfSubpCall = 1009,
    vhpiCbAfterDelay = 1010,
    vhpiCbRepAfterDelay = 1011,
    vhpiCbNextTimeStep = 1012,
    vhpiCbRepNextTimeStep = 1013,
    vhpiCbStartOfNextCycle = 1014,
    vhpiCbRepStartOfNextCycle = 1015,
    vhpiCbStartOfProcesses = 1016,
    vhpiCbRepStartOfProcesses = 1017,
    vhpiCbEndOfProcesses = 1018,
    vhpiCbRepEndOfProcesses = 1019,
    vhpiCbLastKnownDeltaCycle = 1020,
    vhpiCbRepLastKnownDeltaCycle = 1021,
    vhpiCbStartOfPostponed = 1022,
    vhpiCbRepStartOfPostponed = 1023,
    vhpiCbEndOfTimeStep = 1024,
    vhpiCbRepEndOfTimeStep = 1025,
    vhpiCbStartOfTool = 1026,
    vhpiCbEndOfTool = 1027,
    vhpiCbStartOfAnalysis = 1028,
    vhpiCbEndOfAnalysis = 1029,
    vhpiCbStartOfElaboration = 1030,
    vhpiCbEndOfElaboration = 1031,
    vhpiCbStartOfInitialization = 1032,
    vhpiCbEndOfInitialization = 1033,
    vhpiCbStartOfSimulation = 1034,
    vhpiCbEndOfSimulation = 1035,
    vhpiCbQuiescense = 1036,
    vhpiCbPLIError = 1037,
    vhpiCbStartOfSave = 1038,
    vhpiCbEndOfSave = 1039,
    vhpiCbStartOfRestart = 1040,
    vhpiCbEndOfRestart = 1041,
    vhpiCbStartOfReset = 1042,
    vhpiCbEndOfReset = 1043,
    vhpiCbEnterInteractive = 1044,
    vhpiCbExitInteractive = 1045,
    vhpiCbSigInterrupt = 1046,
    vhpiCbTimeOut = 1047,
    vhpiCbRepTimeOut = 1048,
    vhpiCbSensitivity = 1049,
    vhpiReturnCb = 0x00000001,
    vhpiDisableCb = 0x00000010,
} }

/// Every constant group, for ABI tests.
pub const ALL_VALUES: &[&[(&str, i64)]] = &[
    MISC_VALUES,
    VHPIFORMATT_VALUES,
    VHPICLASSKINDT_VALUES,
    VHPIONETOONET_VALUES,
    VHPIONETOMANYT_VALUES,
    VHPIINTPROPERTYT_VALUES,
    VHPISTRPROPERTYT_VALUES,
    VHPIREALPROPERTYT_VALUES,
    VHPIPHYSPROPERTYT_VALUES,
    VHPICAPABIBILITYT_VALUES,
    VHPIOPENMODET_VALUES,
    VHPIMODET_VALUES,
    VHPISIGKINDT_VALUES,
    VHPISTATICNESST_VALUES,
    VHPIPREDEFATTRT_VALUES,
    VHPIATTRKINDT_VALUES,
    VHPIENTITYCLASST_VALUES,
    VHPIACCESST_VALUES,
    VHPISTATET_VALUES,
    VHPICOMPINSTKINDT_VALUES,
    VHPIPHASET_VALUES,
    VHPISEVERITYT_VALUES,
    VHPIAUTOMATICRESTORET_VALUES,
    VHPIPUTVALUEMODET_VALUES,
    VHPIDELAYMODET_VALUES,
    VHPISIMCONTROLT_VALUES,
    VHPIFOREIGNKINDT_VALUES,
    DEFINE_VALUES,
];
