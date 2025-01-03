enum AlertType {
    Update = 'Update',
    New = 'New',
    Remove = 'Remove'
}

interface SentinelAlert {
    id: string;
    atype: AlertType;
    name: string;
    performance: number;
    expected: number;
    up: boolean;
    reason: string;
    error?: string;
}

export type { SentinelAlert, AlertType };
