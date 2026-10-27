-- 0001 init
-- Written by `kiln db diff` for sqlite. Read it before applying it.

create table accounts (
    id integer primary key,
    name text not null,
    email text not null,
    password text not null,
    verified integer not null,
    role text not null
);

create table events (
    id integer primary key,
    account_id integer not null,
    action text not null,
    subject text not null,
    at text not null
);

create table releases (
    id integer primary key,
    account_id integer not null,
    publisher text not null,
    name text not null,
    version text not null,
    checksum text not null,
    description text not null,
    license text not null,
    dependencies text not null,
    capabilities text not null,
    archive text not null,
    yanked integer not null,
    published_at text not null
);

create table reviews (
    id integer primary key,
    publisher text not null,
    name text not null,
    version text not null,
    checksum text not null,
    reviewer text not null,
    notes text not null
);

create table tokens (
    id integer primary key,
    account_id integer not null references accounts(id),
    token_hash text not null,
    label text not null
);

