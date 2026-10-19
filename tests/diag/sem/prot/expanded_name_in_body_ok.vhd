-- yavs-test: std=2008
-- expect: none
-- LRM 8.3, 12.1: inside a protected type declaration and its body (one declarative region) an
-- expanded name with the type as prefix denotes the state or method declared there (#250).
package p is
  type prot_t is protected
    procedure set (n : integer);
    impure function get return integer;
    impure function twice return integer;
  end protected;
end package;
package body p is
  type prot_t is protected body
    variable n : integer := 0;
    procedure set (n : integer) is
    begin
      prot_t.n := n;
    end procedure;
    impure function get return integer is
    begin
      return prot_t.n;
    end function;
    impure function twice return integer is
    begin
      return 2 * prot_t.get;
    end function;
  end protected body;
end package body;
