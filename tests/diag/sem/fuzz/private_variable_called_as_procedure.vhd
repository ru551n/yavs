-- yavs-test: std=2019
-- expect: E1361 14:7 this name is not a procedure
-- LRM 5.6.2 (2019): a private variable is no procedure; reached the invariant check of no_method (#535)
package p is
  type pt is protected
    private variable cnt : integer;
    procedure inc (other : inout pt);
  end protected;
end package;
package body p is
  type pt is protected body
    procedure inc (other : inout pt) is
    begin
      other.cnt;
    end procedure;
  end protected body;
end package body;
