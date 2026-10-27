-- 0001 init
-- Written by `kiln db diff` for sqlite. Read it before applying it.

create table posts (
    id integer primary key,
    title text not null,
    body text not null,
    published integer not null
);

