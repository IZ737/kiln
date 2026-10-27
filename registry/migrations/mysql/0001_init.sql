-- 0001 init
-- Written by `kiln db diff` for mysql. Read it before applying it.

create table accounts (
    id bigint auto_increment primary key,
    name text not null,
    email text not null,
    password text not null,
    verified boolean not null,
    role text not null
);

create table events (
    id bigint auto_increment primary key,
    account_id bigint not null,
    action text not null,
    subject text not null,
    at text not null
);

create table releases (
    id bigint auto_increment primary key,
    account_id bigint not null,
    publisher text not null,
    name text not null,
    version text not null,
    checksum text not null,
    description text not null,
    license text not null,
    dependencies text not null,
    capabilities text not null,
    archive text not null,
    yanked boolean not null,
    published_at text not null
);

create table reviews (
    id bigint auto_increment primary key,
    publisher text not null,
    name text not null,
    version text not null,
    checksum text not null,
    reviewer text not null,
    notes text not null
);

create table tokens (
    id bigint auto_increment primary key,
    account_id bigint not null,
    token_hash text not null,
    label text not null,
    foreign key (account_id) references accounts(id)
);

