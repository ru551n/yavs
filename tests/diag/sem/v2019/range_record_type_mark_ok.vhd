-- yavs-test: std=2019
-- expect: none
-- LRM 16.2.6 (2019): R'RECORD is the range record type of the range R (#180)
package p is
  constant r : natural'range'record := (left => 0, right => 3, direction => ascending);
  type m_t is protected
    procedure clear (r : natural'range'record);
  end protected;
end package;
package body p is
  type m_t is protected body
    procedure clear (r : natural'range'record) is
    begin
      assert r.left <= r.right;
    end procedure;
  end protected body;
end package body;
