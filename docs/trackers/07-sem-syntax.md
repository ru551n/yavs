Open issues where analysis accepts invalid code, rejects valid code, or reports the wrong diagnostic. Decisions on language-revision behaviour are listed with the bugs they affect.

## Issues
- [ ] #867 sem: a package body inside a package declaration is accepted
- [ ] #861 decision: deferred constant used in its package body before the full declaration, analysis error or run-time error
- [ ] #860 decision: null range choice next to other associations in an aggregate, 1993 only or every revision
- [ ] #841 syntax/sem: PSL: bare && and & between sequences give W0004/E1340; W0004 note text narrower than what runs; -- psl comments in a 93 file with --psl give E0002
- [ ] #820 sem: incremental binding with a generic map only reports the generics of the primary binding as unassociated (VESTs Ashenden ch_13_fg_13_24)
- [ ] #819 sem: an entity aspect naming an architecture analysed later is rejected at analysis (VESTs Ashenden ch_11_fg_11_09, ch_13_ch_13_01)
- [x] #658 sem: a user attribute of a subtype as a choice of an aggregate is E1621
- [x] #657 sem: indexing or slicing the value of a user attribute of a type or subtype (mem'xa(1)) is E1621
- [x] #628 sem: a constant and a subprogram with the same designator, both potentially visible by use clauses, are not hidden (LRM 12.4); accepted in strict 2008
- [x] #621 sem: HIR validation in debug builds rejects valid code (2-D aggregate with named choices, nested procedure of a generic subprogram instance)
- [ ] #617 sem: a type conversion with a slice as the formal part of a port association is rejected
- [x] #616 syntax: 'subtype of an indexed name as a type mark is a parse error
- [x] #614 sem: a binding indication with a generic map of a type shares the entity's type between two instances
- [x] #607 sem: an access subtype with an index constraint is E1211
- [ ] #605 sem: a package body of a package with a generic map is E9999
- [ ] #44 sem: names/units wiring leftovers and fuzz regression guard
