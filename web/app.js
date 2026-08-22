const PAGE_SIZE = 50;
const INTEGER_PATTERN = /^-?\d+$/;

const state = {
  profiles: [],
  scope: null,
  driverSnapshots: new Map(),
  driverStats: [],
  selectedDriver: null,
  offset: 0,
  refreshGeneration: 0,
};

const el = {
  notice: document.querySelector('#notice'),
  dashboard: document.querySelector('#dashboard'),
  emptyState: document.querySelector('#empty-state'),
  profileWrap: document.querySelector('#profile-wrap'),
  profileSelect: document.querySelector('#profile-select'),
  summary: document.querySelector('#summary'),
  topDrivers: document.querySelector('#top-drivers'),
  losingDrivers: document.querySelector('#losing-drivers'),
  driverStatus: document.querySelector('#driver-status'),
  drivers: document.querySelector('#drivers'),
  driverDetail: document.querySelector('#driver-detail'),
  driverDetailTitle: document.querySelector('#driver-detail-title'),
  driverDetailMeta: document.querySelector('#driver-detail-meta'),
  driverTrips: document.querySelector('#driver-trips'),
  closeDriver: document.querySelector('#close-driver'),
  previous: document.querySelector('#previous'),
  next: document.querySelector('#next'),
  pageLabel: document.querySelector('#page-label'),
  refresh: document.querySelector('#refresh'),
};

export function formatInteger(value) {
  if (typeof value !== 'string' || !INTEGER_PATTERN.test(value)) return '—';
  const negative = value.startsWith('-');
  const digits = negative ? value.slice(1) : value;
  const grouped = digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return negative ? `-${grouped}` : grouped;
}

function node(tag, text, className = '') {
  const item = document.createElement(tag);
  item.textContent = text;
  if (className) item.className = className;
  return item;
}

function clear(element) {
  element.replaceChildren();
}

function showNotice(message, error = false) {
  el.notice.textContent = message;
  el.notice.classList.toggle('error', error);
}

function safeText(value, fallback = '—') {
  return typeof value === 'string' && value !== '' ? value : fallback;
}

function requireInteger(value) {
  if (typeof value !== 'string' || !INTEGER_PATTERN.test(value)) {
    throw new Error('TruckLedger returned unexpected numeric data.');
  }
  return value;
}

function requireCount(value) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error('TruckLedger returned unexpected count data.');
  }
  return value;
}

function toBigInt(value) {
  return BigInt(requireInteger(value));
}

function friendlyDriver(rawId) {
  if (typeof rawId !== 'string' || rawId === '') return 'Driver';
  const match = /^driver\.(.+)$/.exec(rawId);
  return match ? `Driver ${match[1]}` : rawId;
}

function compareRawId(left, right) {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

function signedInteger(value) {
  const raw = requireInteger(value);
  if (/^-?0+$/.test(raw)) return formatInteger(raw);
  return raw.startsWith('-') ? formatInteger(raw) : `+${formatInteger(raw)}`;
}

function netClass(value) {
  const net = toBigInt(value);
  return net > 0n ? 'net-positive' : net < 0n ? 'net-negative' : 'number';
}

async function api(path) {
  let response;

  try {
    response = await fetch(path, {
      headers: { Accept: 'application/json' },
      cache: 'no-store',
    });
  } catch (_) {
    throw new Error('TruckLedger server could not be reached.');
  }

  let body;
  try {
    body = await response.json();
  } catch (_) {
    throw new Error('TruckLedger returned an unexpected response.');
  }

  if (!response.ok) {
    if (response.status === 404) {
      throw new Error('Selected profile is no longer available. Refresh and try again.');
    }
    if (response.status === 503) {
      throw new Error('TruckLedger database is temporarily unavailable.');
    }
    throw new Error('TruckLedger could not load saved history.');
  }

  return body;
}

function validProfiles(body) {
  if (
    !body ||
    !Array.isArray(body.profiles) ||
    body.profiles.some(
      (profile) =>
        !profile ||
        typeof profile.scope_key !== 'string' ||
        !Number.isSafeInteger(profile.driver_count) ||
        !Number.isSafeInteger(profile.trip_count),
    )
  ) {
    throw new Error('TruckLedger returned unexpected profile data.');
  }

  return body.profiles;
}

function addStat(label, value, className = '') {
  const item = document.createElement('div');
  item.className = `stat${className ? ` ${className}` : ''}`;
  item.append(
    node('span', label, 'stat-label'),
    node('strong', value, 'stat-value'),
  );
  el.summary.append(item);
}

function renderSummary(summary) {
  if (!summary || typeof summary !== 'object') {
    throw new Error('TruckLedger returned unexpected summary data.');
  }

  clear(el.summary);

  addStat('Trips', String(requireCount(summary.trip_count)));
  addStat('Drivers', String(requireCount(summary.hired_driver_count)));
  addStat('Distance', formatInteger(requireInteger(summary.total_distance)));
  addStat('Net', signedInteger(requireInteger(summary.total_net)), 'net');
}

function loadDriverSnapshots(body) {
  if (!body || !Array.isArray(body.drivers)) {
    throw new Error('TruckLedger returned unexpected driver data.');
  }

  state.driverSnapshots = new Map();

  for (const driver of body.drivers) {
    if (!driver || typeof driver.raw_id !== 'string') {
      throw new Error('TruckLedger returned unexpected driver data.');
    }
    state.driverSnapshots.set(driver.raw_id, driver);
  }
}

function loadDriverStats(body) {
  if (!body || !Array.isArray(body.drivers)) {
    throw new Error('TruckLedger returned unexpected driver statistics.');
  }
  return body.drivers.map((stat) => {
    if (!stat || typeof stat.raw_id !== 'string') {
      throw new Error('TruckLedger returned unexpected driver statistics.');
    }
    const trips = requireCount(stat.trip_count);
    const loaded = requireCount(stat.loaded_trip_count);
    const empty = requireCount(stat.empty_trip_count);
    if (loaded + empty !== trips) {
      throw new Error('TruckLedger returned inconsistent driver statistics.');
    }
    return {
      rawId: stat.raw_id,
      trips,
      loaded,
      empty,
      distance: toBigInt(stat.total_distance),
      revenue: toBigInt(stat.total_revenue),
      wage: toBigInt(stat.total_wage),
      maintenance: toBigInt(stat.total_maintenance),
      fuel: toBigInt(stat.total_fuel),
      costs: toBigInt(stat.total_costs),
      net: toBigInt(stat.total_net),
    };
  });
}

function openDriver(rawId) {
  state.selectedDriver = rawId;
  state.offset = 0;
  loadSelectedDriverTrips()
    .then(() => {
      el.driverDetail.hidden = false;
      el.driverDetail.scrollIntoView({ behavior: 'smooth', block: 'start' });
    })
    .catch((error) => showNotice(error.message, true));
}

function rankingButton(stat, rank, positive) {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'ranking-row';
  button.addEventListener('click', () => openDriver(stat.rawId));

  const main = document.createElement('div');
  main.className = 'ranking-main';
  main.append(
    node('div', friendlyDriver(stat.rawId), 'ranking-name'),
    node(
      'div',
      `${stat.trips} trips · ${stat.loaded} loaded · ${stat.empty} empty`,
      'ranking-meta',
    ),
  );

  button.append(
    node('span', String(rank), 'rank'),
    main,
    node(
      'strong',
      `${stat.net > 0n ? '+' : ''}${formatInteger(stat.net.toString())}`,
      `ranking-net ${positive ? 'positive' : 'negative'}`,
    ),
  );

  return button;
}

function renderRankings() {
  clear(el.topDrivers);
  clear(el.losingDrivers);

  const top = state.driverStats
    .filter((stat) => stat.net > 0n)
    .sort((a, b) => {
      if (a.net > b.net) return -1;
      if (a.net < b.net) return 1;
      return compareRawId(a.rawId, b.rawId);
    })
    .slice(0, 3);

  const losses = state.driverStats
    .filter((stat) => stat.net < 0n)
    .sort((a, b) => {
      if (a.net < b.net) return -1;
      if (a.net > b.net) return 1;
      return compareRawId(a.rawId, b.rawId);
    })
    .slice(0, 5);

  if (top.length === 0) {
    el.topDrivers.append(node('p', 'No profitable drivers yet.', 'no-ranking'));
  } else {
    top.forEach((stat, index) => el.topDrivers.append(rankingButton(stat, index + 1, true)));
  }

  if (losses.length === 0) {
    el.losingDrivers.append(node('p', 'No drivers are currently below zero.', 'no-ranking'));
  } else {
    losses.forEach((stat, index) => el.losingDrivers.append(rankingButton(stat, index + 1, false)));
  }
}

function appendCell(row, label, content, className = '') {
  const cell = document.createElement('td');
  cell.dataset.label = label;
  if (className) cell.className = className;

  if (content instanceof Node) cell.append(content);
  else cell.textContent = content;

  row.append(cell);
}

function renderAllDrivers() {
  clear(el.drivers);

  el.driverStatus.textContent =
    `${state.driverStats.length} driver${state.driverStats.length === 1 ? '' : 's'} · click a driver to inspect trip history.`;

  if (state.driverStats.length === 0) {
    const row = document.createElement('tr');
    const cell = node('td', 'No hired drivers archived for this profile.', 'empty-row');
    cell.colSpan = 8;
    row.append(cell);
    el.drivers.append(row);
    return;
  }

  const ordered = [...state.driverStats].sort((a, b) => {
    if (a.net > b.net) return -1;
    if (a.net < b.net) return 1;
    return compareRawId(a.rawId, b.rawId);
  });

  for (const stat of ordered) {
    const row = document.createElement('tr');

    const driverButton = node('button', friendlyDriver(stat.rawId), 'driver-link');
    driverButton.type = 'button';
    driverButton.addEventListener('click', () => openDriver(stat.rawId));

    appendCell(row, 'Driver', driverButton);
    appendCell(row, 'Trips', String(stat.trips), 'number');
    appendCell(row, 'Loaded', String(stat.loaded), 'number');
    appendCell(row, 'Empty', String(stat.empty), 'number');
    appendCell(row, 'Distance', formatInteger(stat.distance.toString()), 'number');
    appendCell(row, 'Revenue', formatInteger(stat.revenue.toString()), 'number');
    appendCell(row, 'Costs', formatInteger(stat.costs.toString()), 'number');
    appendCell(
      row,
      'Net',
      `${stat.net > 0n ? '+' : ''}${formatInteger(stat.net.toString())}`,
      stat.net > 0n ? 'net-positive' : stat.net < 0n ? 'net-negative' : 'number',
    );

    el.drivers.append(row);
  }
}

function routeNode(trip) {
  const source = safeText(trip.source_city, safeText(trip.source_company, 'Unknown'));
  const destination = safeText(trip.destination_city, safeText(trip.destination_company, 'Unknown'));

  const wrapper = document.createElement('div');
  wrapper.className = 'route';
  wrapper.append(node('div', `${source} → ${destination}`, 'route-main'));

  const sourceCompany = trip.source_city ? safeText(trip.source_company, '') : '';
  const destinationCompany = trip.destination_city ? safeText(trip.destination_company, '') : '';
  const companies = [sourceCompany, destinationCompany].filter(Boolean).join(' → ');

  if (companies) wrapper.append(node('div', companies, 'route-sub'));

  return wrapper;
}

function renderDriverTrips(body) {
  if (
    !body ||
    !Array.isArray(body.trips) ||
    body.limit !== PAGE_SIZE ||
    body.offset !== state.offset
  ) {
    throw new Error('TruckLedger returned unexpected trip data.');
  }

  clear(el.driverTrips);

  if (body.trips.length === 0) {
    const row = document.createElement('tr');
    const cell = node('td', 'No archived trips for this driver.', 'empty-row');
    cell.colSpan = 10;
    row.append(cell);
    el.driverTrips.append(row);
  }

  for (const trip of body.trips) {
    const row = document.createElement('tr');
    const activity = trip.distance_on_job ? 'Loaded' : 'Empty';

    appendCell(row, 'Day', `Day ${requireInteger(trip.timestamp_day)}`);
    appendCell(row, 'Route', routeNode(trip), 'route');
    appendCell(row, 'Cargo', safeText(trip.cargo, '—'), 'cargo');
    appendCell(row, 'Activity', activity, `activity ${trip.distance_on_job ? 'loaded' : 'empty'}`);
    appendCell(row, 'Distance', formatInteger(requireInteger(trip.distance)), 'number');
    appendCell(row, 'Revenue', formatInteger(requireInteger(trip.revenue)), 'number');
    appendCell(row, 'Wage', formatInteger(requireInteger(trip.wage)), 'number');
    appendCell(row, 'Maintenance', formatInteger(requireInteger(trip.maintenance)), 'number');
    appendCell(row, 'Fuel', formatInteger(requireInteger(trip.fuel)), 'number');
    appendCell(row, 'Net', signedInteger(requireInteger(trip.net)), netClass(trip.net));

    el.driverTrips.append(row);
  }

  const stat = state.driverStats.find((item) => item.rawId === state.selectedDriver);
  const snapshot = state.driverSnapshots.get(state.selectedDriver);

  el.driverDetailTitle.textContent = friendlyDriver(state.selectedDriver);

  const meta = [];
  if (stat) {
    meta.push(`${stat.trips} trips`);
    meta.push(`${stat.loaded} loaded`);
    meta.push(`${stat.empty} empty`);
    meta.push(`Net ${stat.net > 0n ? '+' : ''}${formatInteger(stat.net.toString())}`);
  }
  if (snapshot?.current_city) meta.push(`Current city ${snapshot.current_city}`);

  el.driverDetailMeta.textContent = meta.join(' · ');

  el.previous.disabled = state.offset === 0;
  el.next.disabled = body.trips.length < PAGE_SIZE;
  el.pageLabel.textContent = `Page ${Math.floor(state.offset / PAGE_SIZE) + 1}`;
}

async function loadSelectedDriverTrips() {
  if (!state.scope || !state.selectedDriver) return;

  const scope = encodeURIComponent(state.scope);
  const query = new URLSearchParams({
    limit: String(PAGE_SIZE),
    offset: String(state.offset),
    driver: state.selectedDriver,
  });

  const body = await api(`/api/v1/profiles/${scope}/trips?${query}`);
  renderDriverTrips(body);
}

async function loadProfile() {
  if (!state.scope) return;

  const generation = state.refreshGeneration;
  const scope = encodeURIComponent(state.scope);

  const [summary, drivers, stats] = await Promise.all([
    api(`/api/v1/profiles/${scope}/summary`),
    api(`/api/v1/profiles/${scope}/drivers`),
    api(`/api/v1/profiles/${scope}/driver-stats`),
  ]);

  if (generation !== state.refreshGeneration) return;

  renderSummary(summary);
  loadDriverSnapshots(drivers);
  state.driverStats = loadDriverStats(stats);
  renderRankings();
  renderAllDrivers();

  state.selectedDriver = null;
  state.offset = 0;
  el.driverDetail.hidden = true;
}

async function refresh() {
  const generation = ++state.refreshGeneration;
  el.refresh.disabled = true;
  showNotice('Refreshing…');

  try {
    state.profiles = validProfiles(await api('/api/v1/profiles'));

    if (generation !== state.refreshGeneration) return;

    clear(el.profileSelect);

    for (const profile of state.profiles) {
      const option = node('option', profile.scope_key);
      option.value = profile.scope_key;
      el.profileSelect.append(option);
    }

    if (state.profiles.length === 0) {
      state.scope = null;
      el.profileWrap.hidden = true;
      el.dashboard.hidden = true;
      el.emptyState.hidden = false;
      showNotice('');
      return;
    }

    el.emptyState.hidden = true;
    el.profileWrap.hidden = false;

    if (!state.profiles.some((profile) => profile.scope_key === state.scope)) {
      state.scope = state.profiles[0].scope_key;
    }

    el.profileSelect.value = state.scope;
    await loadProfile();

    if (generation !== state.refreshGeneration) return;

    el.dashboard.hidden = false;
    showNotice('');
  } catch (error) {
    showNotice(
      error instanceof Error ? error.message : 'TruckLedger could not load saved history.',
      true,
    );
  } finally {
    if (generation === state.refreshGeneration) el.refresh.disabled = false;
  }
}

el.profileSelect.addEventListener('change', () => {
  state.scope = el.profileSelect.value;
  state.refreshGeneration += 1;
  state.selectedDriver = null;
  state.offset = 0;

  loadProfile()
    .then(() => showNotice(''))
    .catch((error) => showNotice(error.message, true));
});

el.closeDriver.addEventListener('click', () => {
  state.selectedDriver = null;
  state.offset = 0;
  el.driverDetail.hidden = true;
});

el.previous.addEventListener('click', () => {
  state.offset = Math.max(0, state.offset - PAGE_SIZE);
  loadSelectedDriverTrips().catch((error) => showNotice(error.message, true));
});

el.next.addEventListener('click', () => {
  state.offset += PAGE_SIZE;
  loadSelectedDriverTrips().catch((error) => showNotice(error.message, true));
});

el.refresh.addEventListener('click', refresh);

refresh();
