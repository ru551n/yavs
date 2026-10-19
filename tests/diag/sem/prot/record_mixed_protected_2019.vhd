-- yavs-test: std=2019
-- expect: E1905 9:30 record element `b` is not of a protected type, but another element is
-- LRM 5.3.1 (2019): a record with a protected element can only have protected elements
package p is
  type pt is protected
    procedure inc;
  end protected;
  type ok is record a : pt; c : pt; end record;
  type bad is record a : pt; b : integer; end record;
end package;
package body p is
  type pt is protected body procedure inc is begin end; end protected body;
end package body;
